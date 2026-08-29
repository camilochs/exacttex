//! Command-line front end for the NextTeX compiler.
//!
//! Every host assumption lives here: filesystem paths, the current directory,
//! and process exit codes. `nextex-core` has none of them, which is what lets
//! the same core compile to WebAssembly.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use nextex_core::bibliography::{Bibliography, Unavailable, assemble, declared_in};
use nextex_core::check::{Diagnostic, check};
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
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(first) = args.first() else {
        eprintln!("usage: nextex <file.ntex> | nextex check [--json] [--strict-tex] <file.ntex>");
        eprintln!();
        eprintln!("Emits the file and its imports as LaTeX under build/.");
        return ExitCode::from(2);
    };

    if first == "check" {
        return check_command(&args[1..]);
    }
    let input = first;
    let loader = FileSystem {
        root: PathBuf::from("."),
    };
    let mut sink = BuildDirectory {
        root: PathBuf::from("build"),
    };

    match build(input, &loader, &mut sink) {
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

fn check_command(args: &[String]) -> ExitCode {
    let json = args.iter().any(|arg| arg == "--json");
    let strict = args.iter().any(|arg| arg == "--strict-tex");
    let inputs: Vec<&str> = args
        .iter()
        .filter(|arg| !arg.starts_with("--"))
        .map(String::as_str)
        .collect();
    if inputs.len() != 1 {
        eprintln!("usage: nextex check [--json] [--strict-tex] <file.ntex>");
        return ExitCode::from(2);
    }

    match run_check(inputs[0]) {
        Ok((sources, diagnostics, coverage, bibliography)) => {
            if json {
                print_json(&sources, &diagnostics, coverage);
            } else {
                for diagnostic in &diagnostics {
                    print_human(&sources, diagnostic);
                }
                if strict {
                    if let Bibliography::Unavailable(reason) = bibliography {
                        print_bibliography_advisory(&reason);
                    }
                }
                println!("coverage: {:.1}%", coverage * 100.0);
            }
            if diagnostics.is_empty() {
                ExitCode::SUCCESS
            } else {
                ExitCode::from(1)
            }
        }
        Err(error) => {
            eprintln!("fatal: {error}");
            ExitCode::from(2)
        }
    }
}

fn run_check(root: &str) -> Result<(Sources, Vec<Diagnostic>, f64, Bibliography), IoError> {
    let loader = FileSystem {
        root: PathBuf::from("."),
    };
    let mut sources = Sources::new();
    let root_id = loader.load(root, None, &mut sources)?;
    let document = parse(&sources, root_id);
    let mut table = nextex_core::symbols::SymbolTable::new();
    table.merge(&sources, &document);

    let declared = declared_in(&sources, root_id);
    let base = Path::new(root).parent().unwrap_or_else(|| Path::new("."));
    let bibliography = assemble(&declared, |name| fs::read(base.join(name)).ok());
    let diagnostics = check(&table, &bibliography);
    let coverage = document.coverage();
    Ok((sources, diagnostics, coverage, bibliography))
}

fn location(
    sources: &Sources,
    source: SourceId,
    span: nextex_core::source::Span,
) -> (&str, usize, usize) {
    let Some(source) = sources.get(source) else {
        return ("<unresolved>", 1, 1);
    };
    let before = &source.bytes()[..span.start().min(source.bytes().len())];
    #[allow(clippy::naive_bytecount)]
    let line = before.iter().filter(|byte| **byte == b'\n').count() + 1;
    let column = before
        .iter()
        .rposition(|byte| *byte == b'\n')
        .map_or(before.len() + 1, |at| before.len() - at);
    (source.name(), line, column)
}

fn print_human(sources: &Sources, diagnostic: &Diagnostic) {
    let (file, line, column) = location(sources, diagnostic.source, diagnostic.span);
    println!("error[{}]: {}", diagnostic.code, diagnostic.message);
    println!("  --> {file}:{line}:{column}");
    println!("  entity: {}", diagnostic.entity.name());
    if let Some(name) = &diagnostic.name {
        println!("  name: {name}");
    }
    println!(
        "  span: offset {}, length {}",
        diagnostic.span.start(),
        diagnostic.span.len()
    );
    for related in &diagnostic.related {
        let (file, line, column) = location(sources, related.source, related.span);
        println!("  --> {file}:{line}:{column}: {}", related.message);
    }
    println!("  blame: nextex-construct");
}

fn print_json(sources: &Sources, diagnostics: &[Diagnostic], coverage: f64) {
    print!("{{\"coverage\":{coverage},\"diagnostics\":[");
    for (index, diagnostic) in diagnostics.iter().enumerate() {
        if index > 0 {
            print!(",");
        }
        let (file, line, column) = location(sources, diagnostic.source, diagnostic.span);
        print!(
            "{{\"code\":\"{}\",\"severity\":\"error\",\"blame\":\"nextex-construct\",\"entity\":\"{}\",\"name\":",
            diagnostic.code,
            diagnostic.entity.name()
        );
        match &diagnostic.name {
            Some(name) => print_json_string(name),
            None => print!("null"),
        }
        print!(",\"span\":{{\"file\":");
        print_json_string(file);
        print!(
            ",\"offset\":{},\"length\":{},\"line\":{line},\"column\":{column}}},\"message\":",
            diagnostic.span.start(),
            diagnostic.span.len()
        );
        print_json_string(&diagnostic.message);
        print!(",\"related\":[");
        for (related_index, related) in diagnostic.related.iter().enumerate() {
            if related_index > 0 {
                print!(",");
            }
            let (file, line, column) = location(sources, related.source, related.span);
            print!("{{\"span\":{{\"file\":");
            print_json_string(file);
            print!(
                ",\"offset\":{},\"length\":{},\"line\":{line},\"column\":{column}}},\"message\":",
                related.span.start(),
                related.span.len()
            );
            print_json_string(&related.message);
            print!("}}");
        }
        print!("]}}");
    }
    println!("]}}");
}

fn print_json_string(value: &str) {
    print!("\"");
    for character in value.chars() {
        match character {
            '\"' => print!("\\\""),
            '\\' => print!("\\\\"),
            '\n' => print!("\\n"),
            '\r' => print!("\\r"),
            '\t' => print!("\\t"),
            character if character.is_control() => print!("\\u{:04x}", character as u32),
            character => print!("{character}"),
        }
    }
    print!("\"");
}

fn print_bibliography_advisory(reason: &Unavailable) {
    println!("advisory: citation checking unavailable ({reason:?})");
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
