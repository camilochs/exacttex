//! The exit criterion of #18, run rather than argued.
//!
//! The WebAssembly module must process the awkward transport fixture entirely
//! through caller-supplied buffers, and its emitted bytes and JSON diagnostics
//! must equal the native build's byte for byte.
//!
//! The fixture is chosen to be hostile to a boundary that copies through a
//! string: a Latin-1 `é`, a CRLF, a tab, and a stray `0xFF` that is not valid
//! UTF-8 anywhere. A module that decoded on the way in or out fails here.
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

#[test]
fn wasm_output_equals_native_output_byte_for_byte() {
    let repo = repo();
    if !have("node", &["--version"]) {
        println!("skipped: node is not installed");
        return;
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
        .current_dir(&repo)
        .status();
    if !built.is_ok_and(|status| status.success()) {
        println!("skipped: the wasm32-unknown-unknown target is not installed");
        return;
    }

    let fixture = repo.join("tests/fixtures/wasm/awkward.xtex");
    let source = std::fs::read(&fixture).expect("the fixture");
    assert!(
        std::str::from_utf8(&source).is_err(),
        "the fixture must not be valid UTF-8, or it tests nothing"
    );

    let out = repo.join("target/wasm-parity");
    std::fs::create_dir_all(&out).expect("a place for the outputs");

    let ran = Command::new("node")
        .arg(repo.join("crates/xtex-wasm/tests/parity.mjs"))
        .arg(repo.join("target/wasm32-unknown-unknown/release/xtex_wasm.wasm"))
        .arg(&fixture)
        .arg(&out)
        .status()
        .expect("node runs");
    assert!(ran.success(), "the module did not run");

    // Native, through the same public API the CLI uses.
    let mut sources = xtex_core::source::Sources::new();
    let id = sources.add("document.xtex", source.clone());
    let document = xtex_core::parse(&sources, id);

    let mut native_tex = Vec::new();
    xtex_core::emit(&sources, &document, &mut native_tex).expect("emits");
    let wasm_tex = std::fs::read(out.join("wasm.tex")).expect("the module emitted");
    assert_eq!(
        native_tex, wasm_tex,
        "emitted bytes differ; the Latin-1 byte or the 0xFF did not survive"
    );

    let mut table = xtex_core::symbols::SymbolTable::new();
    table.merge(&sources, &document);
    let diagnostics = xtex_core::check::check(
        &table,
        &xtex_core::bibliography::Bibliography::Unavailable(
            xtex_core::bibliography::Unavailable::NoneDeclared,
        ),
    );
    let mut native_json = String::new();
    xtex_core::check::to_json(
        &sources,
        &diagnostics,
        document.coverage(),
        &mut native_json,
    );
    let wasm_json = std::fs::read_to_string(out.join("wasm.json")).expect("the module checked");
    assert_eq!(
        native_json, wasm_json,
        "JSON differs; coverage precision is the usual cause"
    );
}
