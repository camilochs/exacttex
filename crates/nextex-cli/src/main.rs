//! Command-line front end for the NextTeX compiler.
//!
//! Every host assumption lives here: filesystem paths, the current directory,
//! and process exit codes. `nextex-core` has none of them, which is what lets
//! the same core compile to WebAssembly.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use nextex_core::document::Node;
use nextex_core::io::{IoError, OutputSink, SourceLoader};
use nextex_core::scanner::EntryToken;
use nextex_core::source::{SourceId, Sources};
use nextex_core::{BuildError, emit, parse};

/// Largest source the CLI will read, in bytes.
///
/// A bound rather than a judgement: a runaway input should be refused with a
/// diagnostic rather than exhaust memory.
const MAX_SOURCE_BYTES: u64 = 64 * 1024 * 1024;

/// Reads sources from the filesystem, relative to a project root.
struct FileSystem {
    root: PathBuf,
}

impl FileSystem {
    fn resolve(&self, name: &str, base: Option<&str>) -> PathBuf {
        match base.and_then(|b| Path::new(b).parent().map(Path::to_path_buf)) {
            Some(dir) => self.root.join(dir).join(name),
            None => self.root.join(name),
        }
    }
}

impl SourceLoader for FileSystem {
    fn load(
        &self,
        name: &str,
        relative_to: Option<SourceId>,
        sources: &mut Sources,
    ) -> Result<SourceId, IoError> {
        let base = relative_to.and_then(|id| sources.get(id).map(|s| s.name().to_owned()));
        let path = self.resolve(name, base.as_deref());

        let metadata = fs::metadata(&path).map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => IoError::NotFound {
                name: path.display().to_string(),
            },
            _ => IoError::Unreadable {
                name: path.display().to_string(),
                detail: e.to_string(),
            },
        })?;
        if metadata.len() > MAX_SOURCE_BYTES {
            return Err(IoError::TooLarge {
                name: path.display().to_string(),
                len: usize::try_from(metadata.len()).unwrap_or(usize::MAX),
            });
        }

        let bytes = fs::read(&path).map_err(|e| IoError::Unreadable {
            name: path.display().to_string(),
            detail: e.to_string(),
        })?;

        // The logical name is what the core sees, and it stays root-relative so
        // that a diagnostic never leaks an absolute host path.
        let logical = base
            .and_then(|b| Path::new(&b).parent().map(|d| d.join(name)))
            .unwrap_or_else(|| PathBuf::from(name));
        Ok(sources.add(logical.to_string_lossy().into_owned(), bytes))
    }
}

/// Writes emitted bytes under a build directory.
struct BuildDirectory {
    root: PathBuf,
}

impl OutputSink for BuildDirectory {
    fn write(&mut self, name: &str, bytes: &[u8]) -> Result<(), IoError> {
        let path = self.root.join(name);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| IoError::WriteFailed {
                name: path.display().to_string(),
                detail: e.to_string(),
            })?;
        }
        fs::write(&path, bytes).map_err(|e| IoError::WriteFailed {
            name: path.display().to_string(),
            detail: e.to_string(),
        })
    }
}

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let Some(input) = args.next() else {
        eprintln!("usage: nextex <file.ntex>");
        eprintln!();
        eprintln!("Emits the file and its imports as LaTeX under build/.");
        return ExitCode::from(2);
    };

    let loader = FileSystem {
        root: PathBuf::from("."),
    };
    let mut sink = BuildDirectory {
        root: PathBuf::from("build"),
    };

    match build(&input, &loader, &mut sink) {
        Ok(()) => {
            let mut output = PathBuf::from(&input);
            output.set_extension("tex");
            println!("build/{}", output.display());
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}

fn build(root: &str, loader: &FileSystem, sink: &mut BuildDirectory) -> Result<(), BuildError> {
    let mut pending = vec![root.to_owned()];
    let mut emitted = BTreeSet::new();

    while let Some(name) = pending.pop() {
        if !emitted.insert(name.clone()) {
            continue;
        }
        let mut sources = Sources::new();
        let id = loader.load(&name, None, &mut sources)?;
        let document = parse(&sources, id);
        let source = sources
            .get(id)
            .expect("the loader returned an absent source");

        for node in document.iter() {
            if let Node::Construct {
                kind: EntryToken::Import,
                span,
                ..
            } = node
            {
                let bytes = source.slice(*span).expect("a parsed span left its source");
                let first = bytes.iter().position(|byte| *byte == b'"').unwrap_or(0);
                let last = bytes
                    .iter()
                    .rposition(|byte| *byte == b'"')
                    .unwrap_or(first);
                let import = String::from_utf8_lossy(&bytes[first + 1..last]);
                let relative = Path::new(&name).parent().map_or_else(
                    || PathBuf::from(import.as_ref()),
                    |dir| dir.join(import.as_ref()),
                );
                pending.push(relative.to_string_lossy().into_owned());
            }
        }

        let mut bytes = Vec::new();
        emit(&sources, &document, &mut bytes)?;
        let mut output = PathBuf::from(source.name());
        output.set_extension("tex");
        sink.write(&output.to_string_lossy(), &bytes)?;
    }
    Ok(())
}
