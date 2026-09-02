//! `xtex adopt` on disk: what it writes, what it refuses, and what it prints.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);

struct Case(PathBuf);

impl Case {
    /// A fresh copy of one adopt fixture, without its `expect/` directory.
    fn from_fixture(name: &str) -> Self {
        let id = NEXT.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("xtex-adopt-{}-{id}", std::process::id()));
        let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/adopt")
            .join(name);
        copy_tree(&fixture, &path);
        let _ = fs::remove_dir_all(path.join("expect"));
        Self(path)
    }

    fn adopt(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_xtex"))
            .arg("adopt")
            .args(args)
            .current_dir(&self.0)
            .output()
            .unwrap()
    }

    fn expected(name: &str, file: &str) -> Vec<u8> {
        fs::read(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../tests/fixtures/adopt")
                .join(name)
                .join("expect")
                .join(file),
        )
        .unwrap()
    }
}

impl Drop for Case {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

fn copy_tree(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for entry in fs::read_dir(from).unwrap() {
        let path = entry.unwrap().path();
        let target = to.join(path.file_name().unwrap());
        if path.is_dir() {
            copy_tree(&path, &target);
        } else {
            fs::copy(&path, &target).unwrap();
        }
    }
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

#[test]
fn adopt_writes_beside_the_originals_and_prints_the_report() {
    let case = Case::from_fixture("10-project");
    let out = case.adopt(&["main.tex"]);
    assert!(out.status.success(), "{}", text(&out.stderr));
    assert_eq!(
        text(&out.stdout),
        text(&Case::expected("10-project", "report.txt"))
    );
    for name in ["main", "sections/intro", "sections/method"] {
        assert_eq!(
            fs::read(case.0.join(format!("{name}.xtex"))).unwrap(),
            Case::expected("10-project", &format!("{name}.xtex")),
            "{name}.xtex"
        );
        assert!(
            case.0.join(format!("{name}.tex")).is_file(),
            "{name}.tex must stay"
        );
    }
    assert!(
        !case.0.join("sections/deeper.xtex").exists(),
        "a nested \\input is not followed"
    );
}

#[test]
fn adopt_never_overwrites_an_output_that_exists() {
    let case = Case::from_fixture("10-project");
    fs::write(
        case.0.join("sections/intro.xtex"),
        b"somebody's edited file",
    )
    .unwrap();
    let out = case.adopt(&["main.tex"]);
    assert_eq!(out.status.code(), Some(2));
    assert!(
        text(&out.stderr).contains("sections/intro.xtex already exists"),
        "{}",
        text(&out.stderr)
    );
    // Nothing at all was written: the root's output is absent too.
    assert!(!case.0.join("main.xtex").exists());
    assert_eq!(
        fs::read(case.0.join("sections/intro.xtex")).unwrap(),
        b"somebody's edited file"
    );
}

#[test]
fn in_place_removes_the_tex_after_writing_the_xtex() {
    let case = Case::from_fixture("10-project");
    let out = case.adopt(&["--in-place", "main.tex"]);
    assert!(out.status.success(), "{}", text(&out.stderr));
    for name in ["main", "sections/intro", "sections/method"] {
        assert!(case.0.join(format!("{name}.xtex")).is_file());
        assert!(!case.0.join(format!("{name}.tex")).exists(), "{name}.tex");
    }
    assert!(
        case.0.join("sections/deeper.tex").is_file(),
        "a file the ramp did not convert is not removed"
    );
}

#[test]
fn json_is_the_core_renderer_and_a_left_file_sets_the_exit_code() {
    let case = Case::from_fixture("09-child-left");
    let out = case.adopt(&["--json", "main.tex"]);
    assert_eq!(out.status.code(), Some(1), "{}", text(&out.stderr));
    let store = {
        let mut store = xtex_core::io::Memory::new();
        for name in ["main.tex", "part.tex"] {
            store = store.with_input(name, fs::read(case.0.join(name)).unwrap());
        }
        store
    };
    let adopted = xtex_core::adopt::adopt(&store, "main.tex").unwrap();
    let mut json = String::new();
    xtex_core::adopt::to_json(&adopted, &mut json);
    json.push('\n');
    assert_eq!(text(&out.stdout), json);
    assert!(
        case.0.join("main.xtex").is_file(),
        "the root passed and is written"
    );
    assert!(!case.0.join("part.xtex").exists(), "the child was left");
    assert!(case.0.join("part.tex").is_file());
}

#[test]
fn a_root_that_is_left_writes_nothing() {
    let case = Case::from_fixture("08-already-annotated");
    let out = case.adopt(&["main.tex"]);
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(
        text(&out.stdout),
        text(&Case::expected("08-already-annotated", "report.txt"))
    );
    assert!(!case.0.join("main.xtex").exists());
}

#[test]
fn a_file_that_is_not_tex_is_refused() {
    let case = Case::from_fixture("10-project");
    fs::write(case.0.join("paper.xtex"), b"@ref(x)").unwrap();
    let out = case.adopt(&["paper.xtex"]);
    assert_eq!(out.status.code(), Some(2));
    assert!(
        text(&out.stderr).contains("adopt reads a .tex file"),
        "{}",
        text(&out.stderr)
    );
}
