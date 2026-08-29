//! The exit criteria of #18 and #69, run rather than argued.
//!
//! Two projects go through the WebAssembly module as caller-built bundles, and
//! the results must equal the native tool's byte for byte.
//!
//! The single-file case is the original hostile fixture — a Latin-1 `é`, a
//! CRLF, a tab, and a stray `0xFF` that is not valid UTF-8 anywhere — passed
//! as a one-entry bundle, because a single file is a small project and not a
//! special case. The multi-file case is a project with an `@import`, an
//! author's own `\include`, a bibliography and a figure asset, and its check
//! output is compared against **the CLI binary run on the same project on
//! disk**, which is the strongest form of the claim: two hosts, one answer.
//!
//! The test is skipped, loudly, when the wasm target or Node are absent. A
//! silently skipped test is worse than an absent one.

use std::path::{Path, PathBuf};
use std::process::Command;

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("the repository root")
}

fn have(program: &str, args: &[&str]) -> bool {
    Command::new(program)
        .args(args)
        .output()
        .is_ok_and(|out| out.status.success())
}

/// Builds the module and returns its path, or `None` to skip.
fn built_module(repo: &Path) -> Option<PathBuf> {
    if !have("node", &["--version"]) {
        println!("skipped: node is not installed");
        return None;
    }
    let built = Command::new(env!("CARGO"))
        .args([
            "build",
            "--quiet",
            "-p",
            "xtex-wasm",
            "--target",
            "wasm32-unknown-unknown",
            "--release",
        ])
        .current_dir(repo)
        .status();
    if !built.is_ok_and(|status| status.success()) {
        println!("skipped: the wasm32-unknown-unknown target is not installed");
        return None;
    }
    Some(repo.join("target/wasm32-unknown-unknown/release/xtex_wasm.wasm"))
}

/// Runs the node driver over one project directory.
fn run_module(repo: &Path, module: &Path, project: &Path, root: &str, out: &Path) {
    std::fs::create_dir_all(out).expect("a place for the outputs");
    let ran = Command::new("node")
        .arg(repo.join("crates/xtex-wasm/tests/parity.mjs"))
        .arg(module)
        .arg(project)
        .arg(root)
        .arg(out)
        .status()
        .expect("node runs");
    assert!(ran.success(), "the module did not run");
}

/// A `Memory` loader holding every file under `dir`, under `/`-relative names.
fn memory_of(dir: &Path) -> xtex_core::io::Memory {
    fn walk(store: xtex_core::io::Memory, dir: &Path, base: &Path) -> xtex_core::io::Memory {
        let mut store = store;
        let mut entries: Vec<_> = std::fs::read_dir(dir)
            .expect("the project directory")
            .map(|e| e.expect("an entry").path())
            .collect();
        entries.sort();
        for path in entries {
            if path.is_dir() {
                store = walk(store, &path, base);
            } else {
                let name = path
                    .strip_prefix(base)
                    .expect("inside the project")
                    .to_string_lossy()
                    .replace('\\', "/");
                store = store.with_input(name, std::fs::read(&path).expect("readable"));
            }
        }
        store
    }
    walk(xtex_core::io::Memory::new(), dir, dir)
}

#[test]
fn a_single_file_bundle_equals_the_native_library_byte_for_byte() {
    let repo = repo();
    let Some(module) = built_module(&repo) else {
        return;
    };

    let fixture = repo.join("tests/fixtures/wasm/single/awkward.xtex");
    let source = std::fs::read(&fixture).expect("the fixture");
    assert!(
        std::str::from_utf8(&source).is_err(),
        "the fixture must not be valid UTF-8, or it tests nothing"
    );

    let out = repo.join("target/wasm-parity/single");
    run_module(
        &repo,
        &module,
        &repo.join("tests/fixtures/wasm/single"),
        "awkward.xtex",
        &out,
    );

    let store = memory_of(&repo.join("tests/fixtures/wasm/single"));
    let mut sources = xtex_core::source::Sources::new();
    let id = xtex_core::io::SourceLoader::load(&store, "awkward.xtex", None, &mut sources)
        .expect("loads");
    let document = xtex_core::parse(&sources, id);

    let mut native_tex = Vec::new();
    xtex_core::emit(&sources, &document, &mut native_tex).expect("emits");
    let wasm_tex = std::fs::read(out.join("wasm.tex")).expect("the module emitted");
    assert_eq!(
        native_tex, wasm_tex,
        "emitted bytes differ; the Latin-1 byte or the 0xFF did not survive"
    );

    let (sources, diagnostics, coverage, _) = xtex_core::project::check_project(
        &store,
        "awkward.xtex",
        xtex_core::symbols::PrefixMap::default(),
    )
    .expect("checks");
    let mut native_json = String::new();
    xtex_core::check::to_json(&sources, &diagnostics, coverage, &mut native_json);
    let wasm_json = std::fs::read_to_string(out.join("wasm.json")).expect("the module checked");
    assert_eq!(
        native_json, wasm_json,
        "JSON differs; coverage precision is the usual cause"
    );
}

#[test]
fn a_multi_file_bundle_equals_the_cli_run_on_the_same_project_on_disk() {
    let repo = repo();
    let Some(module) = built_module(&repo) else {
        return;
    };

    let project = repo.join("tests/fixtures/wasm/project");
    let out = repo.join("target/wasm-parity/project");
    run_module(&repo, &module, &project, "main.xtex", &out);

    // The CLI, on the same project, from disk. Two hosts, one answer.
    let cli = Command::new(env!("CARGO"))
        .args([
            "run",
            "--quiet",
            "-p",
            "xtex-cli",
            "--",
            "check",
            "--json",
            "main.xtex",
        ])
        .current_dir(&project)
        .output()
        .expect("the CLI runs");
    let wasm_json = std::fs::read(out.join("wasm.json")).expect("the module checked");
    // The CLI prints a line; the module returns bytes. The newline is the
    // terminal's convention, not part of the answer.
    let mut printed = wasm_json.clone();
    printed.push(b'\n');
    assert_eq!(
        String::from_utf8_lossy(&cli.stdout),
        String::from_utf8_lossy(&printed),
        "the CLI on disk and the module on a bundle disagree about one project"
    );

    // The fixture only proves parity if it exercises the multi-file paths:
    // a resolved cross-file @ref, a citation found in the .bib, and a figure
    // asset that exists. A fixture that stopped doing so would still pass the
    // equality above — both sides would emit the same errors — so the content
    // is pinned here.
    let json = String::from_utf8_lossy(&wasm_json);
    assert!(
        !json.contains("XT1003") && !json.contains("XT1005") && !json.contains("XT1006"),
        "the project fixture must check clean, or parity proves less than it claims: {json}"
    );

    // Emission of the root, against the native library over the same bundle
    // contents.
    let store = memory_of(&project);
    let mut sources = xtex_core::source::Sources::new();
    let id =
        xtex_core::io::SourceLoader::load(&store, "main.xtex", None, &mut sources).expect("loads");
    let document = xtex_core::parse(&sources, id);
    let mut native_tex = Vec::new();
    xtex_core::emit(&sources, &document, &mut native_tex).expect("emits");
    let wasm_tex = std::fs::read(out.join("wasm.tex")).expect("the module emitted");
    assert_eq!(native_tex, wasm_tex, "root emission differs");

    let emission = xtex_core::sourcemap::emit_with_map(&sources, &document).expect("maps");
    let wasm_map = std::fs::read(out.join("wasm.map")).expect("the module mapped");
    assert_eq!(emission.map.to_json(), wasm_map, "source maps differ");
}

#[test]
fn a_bundle_that_lies_about_its_lengths_returns_no_result_and_does_not_crash() {
    let repo = repo();
    let Some(module) = built_module(&repo) else {
        return;
    };
    let out = repo.join("target/wasm-parity/malformed");
    std::fs::create_dir_all(&out).expect("a place for the outputs");
    let ran = Command::new("node")
        .arg(repo.join("crates/xtex-wasm/tests/malformed.mjs"))
        .arg(&module)
        .status()
        .expect("node runs");
    assert!(ran.success(), "a malformed bundle crashed the module");
}
