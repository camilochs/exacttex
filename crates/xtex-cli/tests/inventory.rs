//! `xtex inventory` on disk: the classes it reports and the counts beside them.
//!
//! Two fixtures rather than one, because neither alone carries every case
//! the command has to tell apart. The WebAssembly project has a section, a
//! figure declared by a typed block, a figure declared by an `@id` inside
//! the environment, and declarations in imported files; the float-bodies
//! checking fixture has the `@id` that attaches to nothing and is therefore
//! the unknown open type. Reading a project writes nothing, so both run in
//! place.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn fixture(relative: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .join(relative)
}

fn inventory(project: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_xtex"))
        .arg("inventory")
        .args(args)
        .current_dir(project)
        .output()
        .unwrap()
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

/// The whole listing, pinned. Per-line assertions would still pass with a
/// row missing, a class wrong or the columns unaligned, and every one of
/// those is what a reader of this output is relying on.
#[test]
fn the_plain_listing_names_every_declaration_with_its_class_and_its_uses() {
    let output = inventory(&fixture("wasm/project"), &["main.xtex"]);

    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(
        stdout(&output),
        concat!(
            // An `@id` after a caption inside a figure, referenced by nothing.
            "fig:body     figure   0 references  main.xtex:22:52\n",
            // A figure declared by a typed block.
            "fig:plot     figure   1 reference   main.xtex:11:9\n",
            // Two declarations reached through `@import`, under their own
            // file names: the listing spans the project, not the root.
            "sec:deeper   section  0 references  sections/deeper.xtex:1:24\n",
            // One `@ref` and one `@Cref` name this section, counted as two.
            "sec:intro    section  2 references  main.xtex:3:20\n",
            "sec:model    section  2 references  sections/model.xtex:1:20\n",
            // A `sidewaystable` is a float, so its `@id` is a table.
            "tab:rotated  table    0 references  main.xtex:26:34\n",
        )
    );
}

#[test]
fn an_id_that_attaches_to_nothing_is_reported_as_the_unknown_open_type() {
    let output = inventory(&fixture("checking/10-float-bodies"), &["input.xtex"]);

    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(
        stdout(&output),
        concat!(
            "alg:cap      algorithm     1 reference  input.xtex:13:20\n",
            // `@id(fig:between)` sits in prose between two floats, so no
            // float supplies its class and the prefix does not invent one.
            "fig:between  unknown-open  1 reference  input.xtex:23:31\n",
            "fig:cap      figure        1 reference  input.xtex:4:32\n",
            "fig:end      figure        1 reference  input.xtex:21:5\n",
            "fig:sub      figure        1 reference  input.xtex:18:19\n",
            // Inside a `table*`, so a table however the author named it.
            "fig:wrong    table         1 reference  input.xtex:7:19\n",
            "tab:cell     table         1 reference  input.xtex:9:11\n",
            "tab:opener   table         1 reference  input.xtex:24:18\n",
        )
    );
}

#[test]
fn the_json_is_the_shape_the_wasm_export_returns() {
    let output = inventory(&fixture("wasm/project"), &["--json", "main.xtex"]);

    assert!(output.status.success(), "{}", stderr(&output));
    let json = stdout(&output);
    assert!(
        json.contains(
            "{\"name\":\"fig:plot\",\"class\":\"figure\",\"references\":1,\"span\":{\"file\":\"main.xtex\",\"offset\":264,\"length\":8,\"line\":11,\"column\":9}}"
        ),
        "{json}"
    );
    // Every row the plain form prints, with the same class and the same
    // count — one census read two ways, never two answers.
    let plain = stdout(&inventory(&fixture("wasm/project"), &["main.xtex"]));
    for line in plain.lines() {
        let mut columns = line.split_whitespace();
        let name = columns.next().expect("a name");
        let class = columns.next().expect("a class");
        let count = columns.next().expect("a count");
        assert!(
            json.contains(&format!(
                "{{\"name\":\"{name}\",\"class\":\"{class}\",\"references\":{count},"
            )),
            "{line} is missing from {json}"
        );
    }
}

/// Listing is not checking. `awkward.xtex` has an unresolved `@ref(ghost)`,
/// so `xtex check` exits 1 on it; the census still has to come out.
#[test]
fn a_document_whose_references_do_not_resolve_is_still_listed_and_exits_zero() {
    let project = fixture("wasm/single");
    let check = Command::new(env!("CARGO_BIN_EXE_xtex"))
        .args(["check", "awkward.xtex"])
        .current_dir(&project)
        .output()
        .unwrap();
    assert_eq!(check.status.code(), Some(1), "{}", stdout(&check));

    let output = inventory(&project, &["awkward.xtex"]);

    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    assert_eq!(
        stdout(&output),
        "sec:caf  section  1 reference  awkward.xtex:1:20\n"
    );
}

#[test]
fn a_document_declaring_nothing_says_so_rather_than_printing_an_empty_page() {
    // Every construct in this fixture sits inside a raw escape, so the
    // document declares nothing at all. Silence there is ambiguous — a
    // reader cannot tell it apart from a command that did not run.
    let output = inventory(
        &fixture("raw/01-every-entry-token-inside-a-raw-escape"),
        &["input.xtex"],
    );

    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(stdout(&output), "this document declares no entities\n");
}

#[test]
fn a_bad_invocation_prints_the_usage_line_and_exits_two() {
    let project = fixture("wasm/project");
    for args in [&[][..], &["main.xtex", "sections/model.xtex"][..]] {
        let output = inventory(&project, args);
        assert_eq!(output.status.code(), Some(2), "{args:?}");
        assert!(
            stderr(&output).contains("usage: xtex inventory [--json] <file.xtex>"),
            "{}",
            stderr(&output)
        );
    }

    // A misspelt option is refused rather than ignored: printing the plain
    // form for `--jsonn` answers a question nobody asked.
    let output = inventory(&project, &["--jsonn", "main.xtex"]);
    assert_eq!(output.status.code(), Some(2));
    assert!(
        stderr(&output).contains("unknown option --jsonn"),
        "{}",
        stderr(&output)
    );

    let output = inventory(&project, &["absent.xtex"]);
    assert_eq!(output.status.code(), Some(2));
    assert!(
        stderr(&output).starts_with("error: "),
        "{}",
        stderr(&output)
    );
}
