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
    run_module_with_log(repo, module, project, root, out, None);
}

/// As [`run_module`], optionally handing the driver a stderr and log pair.
fn run_module_with_log(
    repo: &Path,
    module: &Path,
    project: &Path,
    root: &str,
    out: &Path,
    log: Option<(&Path, &Path)>,
) {
    std::fs::create_dir_all(out).expect("a place for the outputs");
    let mut command = Command::new("node");
    command
        .arg(repo.join("crates/xtex-wasm/tests/parity.mjs"))
        .arg(module)
        .arg(project)
        .arg(root)
        .arg(out);
    if let Some((stderr, logfile)) = log {
        command.arg(stderr).arg(logfile);
    }
    let ran = command.status().expect("node runs");
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
fn a_rename_plans_and_applies_identically_and_reports_what_it_left_alone() {
    let repo = repo();
    let Some(module) = built_module(&repo) else {
        return;
    };
    let project = repo.join("tests/fixtures/wasm/project");
    let out = repo.join("target/wasm-parity/project");
    run_module(&repo, &module, &project, "main.xtex", &out);

    let store = memory_of(&project);
    let (sources, documents, names) =
        xtex_core::project::load_imports(&store, "main.xtex").expect("loads");
    let plan = xtex_core::rename::plan(&sources, &documents, "sec:model", "sec:modelo");
    let mut native_json = String::new();
    xtex_core::rename::to_json(&sources, &plan, &mut native_json);
    let wasm_json =
        std::fs::read_to_string(out.join("wasm.rename.json")).expect("the module planned");
    assert_eq!(
        native_json, wasm_json,
        "the two builds plan one rename differently"
    );

    // The fixture only proves what it exercises: an edit in the root, an edit
    // in the imported file where the @id lives, and the \verb occurrence
    // reported untouched with a location an editor can show.
    assert!(
        plan.edits.len() >= 2,
        "cross-file edits are the point: {plan:?}"
    );
    assert_eq!(
        plan.untouched.len(),
        1,
        "the verb occurrence is reported: {plan:?}"
    );
    assert!(
        wasm_json.contains("\"untouched\":[{\"file\":\"main.xtex\""),
        "{wasm_json}"
    );

    // Applying through the module leaves no stale reference: rewrite both
    // files, re-check the rewritten project, and demand silence.
    let renamed_root = std::fs::read(out.join("wasm.renamed.root")).expect("the module applied");
    assert_ne!(
        renamed_root,
        std::fs::read(project.join("main.xtex")).unwrap()
    );
    let mut rewritten = xtex_core::io::Memory::new();
    for (name, bytes) in [
        ("main.xtex", renamed_root.clone()),
        ("sections/model.xtex", {
            let (id, _) = names
                .iter()
                .find(|(_, name)| name == "sections/model.xtex")
                .expect("the imported file");
            xtex_core::rename::apply(sources.get(*id).unwrap().bytes(), *id, &plan)
        }),
    ] {
        rewritten = rewritten.with_input(name, bytes);
    }
    for name in ["appendix.tex", "refs.bib", "figures/plot.pdf"] {
        rewritten = rewritten.with_input(
            name,
            std::fs::read(project.join(name)).expect("fixture file"),
        );
    }
    let (sources2, diagnostics, _, _) = xtex_core::project::check_project(
        &rewritten,
        "main.xtex",
        xtex_core::symbols::PrefixMap::default(),
    )
    .expect("checks");
    let _ = sources2;
    assert!(
        diagnostics.is_empty(),
        "a stale reference survived the rename: {diagnostics:?}"
    );
}

#[test]
fn a_tex_log_translates_identically_in_both_builds_and_never_guesses_blame() {
    let repo = repo();
    let Some(module) = built_module(&repo) else {
        return;
    };

    let project = repo.join("tests/fixtures/wasm/project");
    let stderr_path = repo.join("tests/fixtures/wasm/logs/mixed.stderr");
    let log_path = repo.join("tests/fixtures/wasm/logs/mixed.log");
    let out = repo.join("target/wasm-parity/blame");
    run_module_with_log(
        &repo,
        &module,
        &project,
        "main.xtex",
        &out,
        Some((&stderr_path, &log_path)),
    );
    let wasm_json =
        std::fs::read_to_string(out.join("wasm.blame.json")).expect("the module translated");

    // The native side, through the same core path the CLI prints from.
    let store = memory_of(&project);
    let mut sources = xtex_core::source::Sources::new();
    let id =
        xtex_core::io::SourceLoader::load(&store, "main.xtex", None, &mut sources).expect("loads");
    let document = xtex_core::parse(&sources, id);
    let mut table = xtex_core::symbols::SymbolTable::new();
    table.merge(&sources, &document);
    let emission = xtex_core::sourcemap::emit_with_map(&sources, &document).expect("maps");
    let stderr = std::fs::read_to_string(&stderr_path).expect("the stderr fixture");
    let log = std::fs::read_to_string(&log_path).expect("the log fixture");
    let records = xtex_core::blame::merge_records(Some(&stderr), Some(&log), "main.tex");
    let translated = xtex_core::blame::translate(&records, &emission, &sources, &document, &table);
    let mut native_json = String::new();
    xtex_core::blame::to_json(&translated, &mut native_json);
    assert_eq!(
        native_json, wasm_json,
        "the two builds translate one log differently"
    );

    // The fixture must exercise every shape, or the equality proves less than
    // it claims: a located error, an unrecognised line carried unchanged, an
    // overfull that lands on the figure and names it, and a float warning
    // whose line maps to nothing and must say unresolved rather than guess.
    assert!(wasm_json.contains("\"kind\":\"error\""), "{wasm_json}");
    assert!(
        wasm_json.contains("\"kind\":\"unrecognised\""),
        "{wasm_json}"
    );
    assert!(
        wasm_json.contains("\"name\":\"fig:plot\"") && wasm_json.contains("overflows its line"),
        "the overfull at the figure's line must name the figure: {wasm_json}"
    );
    assert!(
        wasm_json.contains("\"blame\":\"unresolved\""),
        "a line past the emission must be unresolved, never guessed: {wasm_json}"
    );
    assert!(
        wasm_json.contains("Undefined control sequence"),
        "the engine's own words must be carried unchanged: {wasm_json}"
    );
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
