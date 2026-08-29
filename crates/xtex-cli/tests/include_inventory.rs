//! Filesystem-level checks for LaTeX inventory edges.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);

struct Case(PathBuf);

impl Case {
    fn new() -> Self {
        let id = NEXT.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("xtex-include-{}-{id}", std::process::id()));
        fs::create_dir(&path).unwrap();
        Self(path)
    }

    fn write(&self, name: &str, text: &str) {
        let path = self.0.join(name);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, text).unwrap();
    }

    fn check(&self) -> Output {
        Command::new(env!("CARGO_BIN_EXE_xtex"))
            .args(["check", "book.xtex"])
            .current_dir(&self.0)
            .output()
            .unwrap()
    }
}

impl Drop for Case {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

#[test]
fn include_and_input_labels_share_the_root_inventory() {
    let case = Case::new();
    case.write(
        "book.xtex",
        "\\include{chapters/one}\n\\input{chapters/two}\n",
    );
    case.write("chapters/one.tex", "\\label{sec:one}\n");
    case.write("chapters/two.xtex", "@ref(sec:one)\n@ref(sec:missing)\n");

    let output = case.check();

    assert_eq!(output.status.code(), Some(1));
    let diagnostics = stdout(&output);
    assert!(!diagnostics.contains("identifier `sec:one` is not declared"));
    assert!(
        diagnostics.contains("chapters/two.xtex:2:6"),
        "{diagnostics}"
    );
    assert!(diagnostics.contains("identifier `sec:missing` is not declared"));
}

#[test]
fn unreadable_or_quarantined_edges_suppress_missing_reference_errors() {
    for root in [
        "\\include{absent}\n@ref(sec:possibly-there)\n",
        "\\include{dark}\n@ref(sec:possibly-there)\n",
        "\\input{   }\n@ref(sec:possibly-there)\n",
    ] {
        let case = Case::new();
        case.write("book.xtex", root);
        case.write(
            "dark.tex",
            "\\catcode`\\@=11\n\\label{sec:possibly-there}\n",
        );

        let output = case.check();

        assert!(output.status.success(), "{}", stderr(&output));
        assert!(!stdout(&output).contains("XT1003"));
    }
}

#[test]
fn a_missing_import_reports_the_import_but_not_a_possibly_hidden_label() {
    let case = Case::new();
    case.write(
        "book.xtex",
        "@import(\"absent.xtex\")\n@ref(sec:possibly-there)\n",
    );

    let output = case.check();

    assert_eq!(output.status.code(), Some(1));
    assert!(stdout(&output).contains("XT1009"));
    assert!(!stdout(&output).contains("XT1003"));
}

#[test]
fn excluded_includes_are_not_followed() {
    let case = Case::new();
    case.write(
        "book.xtex",
        concat!(
            "% \\include{comment}\n",
            "\\begin{verbatim}\n\\include{verbatim}\n\\end{verbatim}\n",
            "\\newcommand{\\loadit}{\\include{macro-body}}\n",
            "@ref(sec:missing)\n",
        ),
    );

    let output = case.check();

    assert_eq!(output.status.code(), Some(1));
    assert!(stdout(&output).contains("identifier `sec:missing` is not declared"));
}

#[test]
fn a_computed_include_makes_the_inventory_unavailable_without_an_error() {
    for root in [
        "\\include{\\chaptername}\n@ref(sec:possibly-computed)\n",
        "\\input\\othername\n@ref(sec:possibly-computed)\n",
    ] {
        let case = Case::new();
        case.write("book.xtex", root);

        let output = case.check();

        assert!(output.status.success(), "{}", stderr(&output));
        assert!(!stdout(&output).contains("XT1003"));
    }
}

#[test]
fn include_cycles_terminate() {
    let case = Case::new();
    case.write("book.xtex", "\\include{other}\n@ref(sec:other)\n");
    case.write("other.tex", "\\input{book.xtex}\n\\label{sec:other}\n");

    let output = case.check();

    assert!(output.status.success(), "{}", stderr(&output));
    assert!(!stdout(&output).contains("XT1003"));
}

#[test]
fn plain_latex_edges_do_not_change_emission() {
    let case = Case::new();
    let input = "before\\include{chapter}\nafter\n";
    case.write("book.xtex", input);
    case.write("chapter.tex", "chapter\n");

    let output = Command::new(env!("CARGO_BIN_EXE_xtex"))
        .arg("book.xtex")
        .current_dir(&case.0)
        .output()
        .unwrap();

    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(
        fs::read(case.0.join("build/book.tex")).unwrap(),
        input.as_bytes()
    );
    assert!(!Path::new(&case.0.join("build/chapter.tex")).exists());
}
