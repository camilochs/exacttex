//! `xtex compile` and the files the engine has to find (#200).
//!
//! The engine runs beside the emitted file in `build/`, while a figure's `src`
//! is named from the root document's directory. These run a stand-in engine,
//! named in `xtex.toml`, that succeeds only if the figure is where a TeX
//! engine would look for it: beside the file it compiles. So the tests need
//! no TeX installation, and they fail the way the real build failed.
#![cfg(unix)]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);

/// A project in a fresh directory, with the stand-in engine configured.
struct Project(PathBuf);

impl Project {
    fn new(src: &str) -> Self {
        let id = NEXT.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!("xtex-compile-{}-{id}", std::process::id()));
        let project = root.join("project");
        fs::create_dir_all(&project).unwrap();
        // The engine is called from inside build/, and a TeX engine looks for
        // a relative path from there.
        let engine = project.join("engine.sh");
        fs::write(
            &engine,
            format!(
                "#!/bin/sh\ntest -f \"{src}\" || {{ echo \"cannot find {src}\" >&2; exit 1; }}\n"
            ),
        )
        .unwrap();
        fs::set_permissions(&engine, fs::Permissions::from_mode(0o755)).unwrap();
        fs::write(
            project.join("xtex.toml"),
            format!("[tex]\ncommand = \"{}\"\n", engine.display()),
        )
        .unwrap();
        fs::write(
            project.join("paper.xtex"),
            format!(
                "\\documentclass{{article}}\n\\begin{{document}}\nFigure~@ref(fig:bands).\n\\figure(fig:bands) {{\n  src     = \"{src}\"\n  caption = {{Bands}}\n}}\n\\end{{document}}\n"
            ),
        )
        .unwrap();
        Self(root)
    }

    fn project(&self) -> PathBuf {
        self.0.join("project")
    }

    fn write(&self, relative: &str, bytes: &[u8]) {
        let path = self.0.join(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, bytes).unwrap();
    }

    fn run(&self, command: &str) -> Output {
        Command::new(env!("CARGO_BIN_EXE_xtex"))
            .args([command, "paper.xtex"])
            .current_dir(self.project())
            .output()
            .unwrap()
    }
}

impl Drop for Project {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

fn said(output: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

#[test]
fn a_figure_in_its_own_folder_compiles_where_check_finds_it() {
    let project = Project::new("results/bands.png");
    project.write("project/results/bands.png", b"PNG");

    let checked = project.run("check");
    assert!(checked.status.success(), "{}", said(&checked));

    let compiled = project.run("compile");
    assert!(compiled.status.success(), "{}", said(&compiled));
    assert_eq!(
        fs::read(project.project().join("build/results/bands.png")).unwrap(),
        b"PNG"
    );
}

#[test]
fn a_figure_that_is_nowhere_still_fails_and_nothing_is_copied() {
    let project = Project::new("results/missing.png");

    let compiled = project.run("compile");
    assert!(!compiled.status.success());
    assert!(!project.project().join("build/results").exists());
}

#[test]
fn a_path_that_climbs_out_of_build_is_not_copied_there() {
    // `../outside.png` resolves from the root, so the check accepts it; copied
    // to the same relative path beside build/paper.tex it would land in the
    // project directory and overwrite whatever has that name. It is left for
    // the engine to report instead.
    let project = Project::new("../outside.png");
    project.write("outside.png", b"PNG");
    project.write("project/outside.png", b"THE AUTHOR'S OWN FILE");

    let checked = project.run("check");
    assert!(checked.status.success(), "{}", said(&checked));
    let _ = project.run("compile");

    assert_eq!(
        fs::read(project.project().join("outside.png")).unwrap(),
        b"THE AUTHOR'S OWN FILE"
    );
}

#[test]
fn a_link_inside_build_does_not_redirect_the_copy() {
    // Anything already in build/ may be a symbolic link, and copying through
    // one writes wherever it points. Found in review: a lexical check on the
    // path alone let `build/results -> outside` take the copy out of build/.
    let project = Project::new("results/bands.png");
    project.write("project/results/bands.png", b"PNG");
    fs::create_dir_all(project.0.join("outside")).unwrap();
    fs::create_dir_all(project.project().join("build")).unwrap();
    std::os::unix::fs::symlink(
        project.0.join("outside"),
        project.project().join("build/results"),
    )
    .unwrap();

    let _ = project.run("compile");

    assert!(!project.0.join("outside/bands.png").exists());
}

#[test]
fn a_link_at_the_destination_does_not_overwrite_what_it_points_to() {
    let project = Project::new("results/bands.png");
    project.write("project/results/bands.png", b"PNG");
    project.write("notes.txt", b"THE AUTHOR'S NOTES");
    fs::create_dir_all(project.project().join("build/results")).unwrap();
    std::os::unix::fs::symlink(
        project.0.join("notes.txt"),
        project.project().join("build/results/bands.png"),
    )
    .unwrap();

    let _ = project.run("compile");

    assert_eq!(
        fs::read(project.0.join("notes.txt")).unwrap(),
        b"THE AUTHOR'S NOTES"
    );
}
