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
///
/// `out` must belong to one test. Tests run on parallel threads, and the
/// driver truncates and rewrites every file it produces; two tests sharing
/// a directory read each other's half-written files — the revision-views
/// test failed about once in six full runs with an empty `original` view,
/// and passed alone every time. One directory per test is the fix; a mutex
/// would serialise work that has no reason to wait.
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

    let (sources, diagnostics, coverage, bibliography) = xtex_core::project::check_project(
        &store,
        "awkward.xtex",
        xtex_core::symbols::PrefixMap::default(),
    )
    .expect("checks");
    let mut native_json = String::new();
    xtex_core::check::to_json(
        &sources,
        &diagnostics,
        coverage,
        &bibliography,
        &mut native_json,
    );
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
fn editor_queries_answer_project_wide_and_identically_in_both_builds() {
    let repo = repo();
    let Some(module) = built_module(&repo) else {
        return;
    };
    let project = repo.join("tests/fixtures/wasm/project");
    let out = repo.join("target/wasm-parity/editor");
    run_module(&repo, &module, &project, "main.xtex", &out);

    let store = memory_of(&project);
    let analysed = xtex_core::project::analyse(&store, "main.xtex").expect("loads");
    let document = &analysed.documents[0];
    let root_text = std::fs::read_to_string(project.join("main.xtex")).expect("the root");
    let ref_at = root_text.find("@ref(sec:model)").expect("the ref") + 6;

    // Hover, over a name declared in the IMPORTED file — the project-wide
    // half is the claim, so the assertion demands the declaration was seen.
    let found = xtex_core::editor::hover(&analysed.sources, document, &analysed.table, ref_at)
        .expect("hover");
    let mut native = String::new();
    xtex_core::editor::hover_to_json(&found, &mut native);
    let wasm = std::fs::read_to_string(out.join("wasm.hover.json")).expect("the module hovered");
    assert_eq!(native, wasm);
    assert!(
        wasm.contains("declared as: section"),
        "the cross-file declaration must be visible, or the table is file-local: {wasm}"
    );

    // The inventory: every declaration with class, site and use count, and
    // the module's answer byte-identical to the native library's.
    let mut native_inventory = String::new();
    xtex_core::editor::inventory_to_json(&analysed.sources, &analysed.table, &mut native_inventory);
    let wasm_inventory =
        std::fs::read_to_string(out.join("wasm.inventory.json")).expect("the module inventoried");
    assert_eq!(native_inventory, wasm_inventory);
    assert!(
        wasm_inventory.contains("\"references\":"),
        "counts must be present: {wasm_inventory}"
    );

    // Completions, which must include identifiers from every file.
    let items =
        xtex_core::editor::completions(&analysed.sources, document, &analysed.table, ref_at);
    let mut native = String::new();
    xtex_core::editor::completions_to_json(&items, &mut native);
    let wasm =
        std::fs::read_to_string(out.join("wasm.completions.json")).expect("the module completed");
    assert_eq!(native, wasm);
    // `sec:model` is declared in the imported file, so its presence is the
    // project-wide claim. `fig:plot` must be ABSENT: the position is inside
    // `@ref(sec:…)`, whose prefix demands a section (`decisions/0003`), and
    // offering a figure there would be the compiler ignoring its own rule.
    assert!(
        wasm.contains("sec:model") && wasm.contains("sec:intro"),
        "completions must span the project: {wasm}"
    );
    assert!(
        !wasm.contains("fig:plot"),
        "the sec: prefix demands a section; a figure offered here breaks decisions/0003: {wasm}"
    );

    // Definition, landing in the imported file — the answer carries the file.
    let (source, span) =
        xtex_core::editor::definition_site(&analysed.sources, document, &analysed.table, ref_at)
            .expect("definition");
    let mut native = String::new();
    xtex_core::editor::definition_to_json(&analysed.sources, source, span, &mut native);
    let wasm =
        std::fs::read_to_string(out.join("wasm.definition.json")).expect("the module defined");
    assert_eq!(native, wasm);
    assert!(
        wasm.contains("\"file\":\"sections/model.xtex\""),
        "a definition in another file must say which: {wasm}"
    );

    // A cleveref reference naming two identifiers: the hover and the
    // definition are about the key under the cursor, and both builds agree.
    let cref_at = root_text
        .find("@Cref(sec:model, sec:intro)")
        .expect("the cref")
        + 7;
    let found = xtex_core::editor::hover(&analysed.sources, document, &analysed.table, cref_at)
        .expect("hover on the cref");
    let mut native = String::new();
    xtex_core::editor::hover_to_json(&found, &mut native);
    let wasm =
        std::fs::read_to_string(out.join("wasm.hover.cref.json")).expect("the module hovered");
    assert_eq!(native, wasm);
    assert!(
        wasm.contains("@Cref(sec:model)") && wasm.contains("declared as: section"),
        "the hover names the command written and the key under the cursor: {wasm}"
    );
    let (source, span) =
        xtex_core::editor::definition_site(&analysed.sources, document, &analysed.table, cref_at)
            .expect("definition of the cref key");
    let mut native = String::new();
    xtex_core::editor::definition_to_json(&analysed.sources, source, span, &mut native);
    let wasm = std::fs::read_to_string(out.join("wasm.definition.cref.json")).expect("ran");
    assert_eq!(native, wasm);
    assert!(wasm.contains("\"file\":\"sections/model.xtex\""), "{wasm}");

    // A position inside opaque text answers nothing rather than a guess, and
    // a position past the end answers nothing rather than trusting it.
    let opaque = std::fs::read(out.join("wasm.hover.opaque.json")).expect("ran");
    assert!(opaque.is_empty(), "opaque text must answer nothing");
    let past = std::fs::read(out.join("wasm.hover.pastend.json")).expect("ran");
    assert!(past.is_empty(), "past the end must answer nothing");

    // And a construct that is not a reference still answers — the control
    // that keeps the two empties above from passing because everything is
    // empty.
    let cite = std::fs::read_to_string(out.join("wasm.hover.cite.json")).expect("ran");
    assert!(cite.contains("citation key"), "{cite}");
}

#[test]
fn a_rename_plans_and_applies_identically_and_reports_what_it_left_alone() {
    let repo = repo();
    let Some(module) = built_module(&repo) else {
        return;
    };
    let project = repo.join("tests/fixtures/wasm/project");
    let out = repo.join("target/wasm-parity/rename");
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
        plan.edits.len() >= 3,
        "cross-file edits are the point, and the cref's key is one of them: {plan:?}"
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
fn revision_views_and_resolutions_cross_between_the_module_and_the_cli() {
    let repo = repo();
    let Some(module) = built_module(&repo) else {
        return;
    };
    let revisions = repo.join("tests/fixtures/wasm/revisions");
    let out = repo.join("target/wasm-parity/revisions");
    run_module(
        &repo,
        &module,
        &repo.join("tests/fixtures/wasm/project"),
        "main.xtex",
        &out,
    );

    // The three views, against the native emitter byte for byte — and the
    // no-injection boundary: only the marked view may differ from a plain
    // emission by injected markup (decisions/0002).
    let store = memory_of(&revisions);
    let mut sources = xtex_core::source::Sources::new();
    let id =
        xtex_core::io::SourceLoader::load(&store, "paper.xtex", None, &mut sources).expect("loads");
    let document = xtex_core::parse(&sources, id);
    for (view, name) in [
        (xtex_core::RevisionView::Original, "original"),
        (xtex_core::RevisionView::Final, "final"),
        (xtex_core::RevisionView::Marked, "marked"),
    ] {
        let mut native = Vec::new();
        xtex_core::emit_view(&sources, &document, view, &mut native).expect("emits");
        let wasm = std::fs::read(out.join(format!("wasm.view.{name}.tex"))).expect("the view");
        assert_eq!(native, wasm, "view {name} differs");
    }
    let original = std::fs::read(out.join("wasm.view.original.tex")).unwrap();
    let final_view = std::fs::read(out.join("wasm.view.final.tex")).unwrap();
    let marked = std::fs::read(out.join("wasm.view.marked.tex")).unwrap();
    assert!(
        !original.windows(4).any(|w| w == b"@add") && !final_view.windows(4).any(|w| w == b"@add"),
        "views are built documents"
    );
    assert!(
        String::from_utf8_lossy(&original).contains("an obsolete clause")
            && !String::from_utf8_lossy(&final_view).contains("an obsolete clause"),
        "the two unmarked views must actually differ over a deletion"
    );
    assert!(
        marked != final_view,
        "the marked view is the one sanctioned injection and must differ"
    );

    // The module's accept, read back by the same parser the CLI uses.
    let pair_bytes = std::fs::read(out.join("wasm.revise.pair")).expect("the module revised");
    let (rewritten, updated_sidecar) = split_pair(&pair_bytes);
    assert!(
        String::from_utf8_lossy(&rewritten).contains("is statistically significant under"),
        "the accepted addition keeps its text"
    );
    assert!(
        !String::from_utf8_lossy(&rewritten).contains("@add(c1)"),
        "the resolved construct is gone"
    );
    let sidecar = xtex_core::review::parse_sidecar(&updated_sidecar)
        .expect("the CLI-side parser reads the module's sidecar");
    xtex_core::review::validate(&rewritten, &sidecar)
        .expect("the module's sidecar validates against the rewritten document");
    let text = String::from_utf8_lossy(&updated_sidecar);
    assert!(
        text.contains("browser-reviewer") && text.contains("resolution = \"accepted\""),
        "the resolution event carries the host-supplied reviewer: {text}"
    );

    // And the reverse: a sidecar the CLI's own resolver wrote is accepted by
    // the module for the next resolution.
    let cli_sidecar = xtex_core::review::resolve_sidecar(
        &std::fs::read(revisions.join("paper.xtexrev")).unwrap(),
        "c1",
        xtex_core::review::Resolution::Accept,
        "cli-reviewer",
        "2026-08-30T11:00:00Z",
        b"",
    )
    .expect("the CLI-side resolver");
    xtex_core::review::parse_sidecar(&cli_sidecar).expect("readable both ways");
}

fn split_pair(bytes: &[u8]) -> (Vec<u8>, Vec<u8>) {
    let first_len = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as usize;
    let first = bytes[4..4 + first_len].to_vec();
    let at = 4 + first_len;
    let second_len =
        u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]]) as usize;
    let second = bytes[at + 4..at + 4 + second_len].to_vec();
    (first, second)
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

#[test]
fn every_export_the_module_declares_is_exercised_by_the_suite() {
    // Coverage by construction: a new export that no parity case touches
    // fails here, so the suite cannot quietly fall behind the surface. The
    // export list is read from the source rather than maintained by hand,
    // because a hand-kept list is the thing that drifts.
    let repo = repo();
    let source = std::fs::read_to_string(repo.join("crates/xtex-wasm/src/lib.rs"))
        .expect("the module's source");
    let mut exports: Vec<&str> = source
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            let rest = line
                .strip_prefix("pub unsafe extern \"C\" fn ")
                .or_else(|| line.strip_prefix("pub extern \"C\" fn "))?;
            rest.split('(').next()
        })
        .collect();
    exports.sort_unstable();
    assert!(!exports.is_empty(), "no exports found; the parser broke");

    let drivers = [
        std::fs::read_to_string(repo.join("crates/xtex-wasm/tests/parity.mjs")).unwrap(),
        std::fs::read_to_string(repo.join("crates/xtex-wasm/tests/malformed.mjs")).unwrap(),
    ];
    let uncovered: Vec<&&str> = exports
        .iter()
        .filter(|name| !drivers.iter().any(|driver| driver.contains(**name)))
        .collect();
    assert!(
        uncovered.is_empty(),
        "exports with no parity case: {uncovered:?}"
    );
}

#[test]
fn failure_paths_fail_identically_in_both_builds() {
    // The builds must agree about failure, not only success: an unreadable
    // bibliography's advisory and a quarantined file's silence are answers
    // too, and a divergence there is found by an author, not by us.
    let repo = repo();
    let Some(module) = built_module(&repo) else {
        return;
    };

    let broken = repo.join("target/wasm-parity/broken-src");
    std::fs::create_dir_all(&broken).expect("a scratch project");
    // An unreadable bibliography: cited, declared, and absent from the
    // bundle. A quarantined import: a \verb that never closes.
    // The duplicate @id makes XT1001 fire, whose diagnostic carries a
    // `related` entry — the shape whose JSON rendering shipped malformed
    // (`{,"span"`) until the first real document met it.
    std::fs::write(
        broken.join("main.xtex"),
        "@id(dup:x) y @id(dup:x).\nVer @cite(clave) y @ref(sec:x).\n@import(\"dark.xtex\")\n\\bibliography{ausente}\n\\table(tab:t) { caption = {T} }\nFigure~@ref(tab:t).\n",
    )
    .expect("writes");
    std::fs::write(
        broken.join("dark.xtex"),
        "\\verb+sin cierre\n\\label{sec:x}\n",
    )
    .expect("writes");

    let out = repo.join("target/wasm-parity/broken");
    run_module(&repo, &module, &broken, "main.xtex", &out);
    let wasm_json = std::fs::read_to_string(out.join("wasm.json")).expect("the module checked");

    let store = memory_of(&broken);
    let (sources, diagnostics, coverage, bibliography) = xtex_core::project::check_project(
        &store,
        "main.xtex",
        xtex_core::symbols::PrefixMap::default(),
    )
    .expect("checks");
    let mut native_json = String::new();
    xtex_core::check::to_json(
        &sources,
        &diagnostics,
        coverage,
        &bibliography,
        &mut native_json,
    );
    assert_eq!(native_json, wasm_json, "the builds disagree about failure");

    // And the failure is the RIGHT failure: the advisory names the missing
    // bibliography, and the quarantined import silences XT1003 rather than
    // blaming the author for a label we could not read.
    assert!(
        wasm_json.contains("XT2001") && wasm_json.contains("ausente"),
        "the advisory must name the file: {wasm_json}"
    );
    assert!(
        !wasm_json.contains("XT1003"),
        "a quarantined file must silence the inventory: {wasm_json}"
    );
    assert!(
        wasm_json.contains("XT1001") && wasm_json.contains("first declared here"),
        "the duplicate must carry its related entry: {wasm_json}"
    );
    // The prose word before a reference is a checked side (decisions/0019),
    // and both builds report it the same way.
    assert!(
        wasm_json.contains("XT1020")
            && wasm_json.contains("prose says `Figure` but `tab:t` is a table"),
        "the prose-word mismatch must be reported: {wasm_json}"
    );
}
