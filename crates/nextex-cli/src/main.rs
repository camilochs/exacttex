//! Command-line front end for the NextTeX compiler.
//!
//! Every host assumption lives here: filesystem paths, the current directory,
//! and process exit codes. `nextex-core` has none of them, which is what lets
//! the same core compile to WebAssembly.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use nextex_core::bibliography::{Bibliography, Declared, Unavailable, assemble, declared_in};
use nextex_core::check::{Diagnostic, check, check_documents};
use nextex_core::diagnostics::map_emitted_diagnostic;
use nextex_core::document::Node;
use nextex_core::io::{IoError, OutputSink, SourceLoader};
use nextex_core::review::{
    Resolution, parse_sidecar, prune_sidecar, resolve, resolve_sidecar, validate,
};
use nextex_core::scanner::EntryToken;
use nextex_core::source::{SourceId, Sources};
use nextex_core::sourcemap::emit_with_map;
use nextex_core::symbols::{EntityClass, PrefixMap, SymbolTable};
use nextex_core::{RevisionView, emit_view, parse};

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

type ResolutionEvent = (String, Resolution, Vec<u8>);
type ResolutionResult = Result<(Vec<u8>, Vec<ResolutionEvent>), String>;

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
        print_usage();
        return ExitCode::from(2);
    };

    if args.first().is_some_and(|arg| arg == "revise") {
        let mut revision_args = args[1..].to_vec();
        if revision_args
            .first()
            .is_some_and(|arg| arg.starts_with("--"))
        {
            let input = match sole_document() {
                Ok(input) => input,
                Err(error) => {
                    eprintln!("error: {error}");
                    return ExitCode::from(2);
                }
            };
            revision_args.insert(0, input);
        }
        return revise(&revision_args);
    }

    if first == "check" {
        return check_command(&args[1..]);
    }
    if first == "blame" {
        return blame(args[1..].iter().cloned());
    }
    let (input, options) = if first == "build" {
        let Some(input) = args.get(1) else {
            print_usage();
            return ExitCode::from(2);
        };
        (input, &args[2..])
    } else {
        (first, &args[1..])
    };
    let loader = FileSystem {
        root: PathBuf::from("."),
    };
    let mut sink = BuildDirectory {
        root: PathBuf::from("build"),
    };

    let mut view = RevisionView::Final;
    for option in options {
        view = match option.as_str() {
            "--original" => RevisionView::Original,
            "--final" => RevisionView::Final,
            "--marked" => RevisionView::Marked,
            _ => {
                eprintln!("error: unknown option {option}");
                return ExitCode::from(2);
            }
        };
    }
    match build(input, view, &loader, &mut sink) {
        Ok(()) => {
            let mut output = PathBuf::from(&input);
            output.set_extension("tex");
            if view == RevisionView::Marked {
                let stem = output.file_stem().unwrap_or_default().to_string_lossy();
                println!("build/{stem}.marked.tex");
            } else {
                println!("build/{}", output.display());
            }
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}

fn print_usage() {
    eprintln!("usage: nextex <file.ntex> [--original|--final|--marked]");
    eprintln!("       nextex build <file.ntex> [--original|--final|--marked]");
    eprintln!("       nextex check [--json] [--strict-tex] <file.ntex>");
    eprintln!("       nextex blame <file.ntex> <line>:<column> [message]");
    eprintln!("       nextex revise <file.ntex> (--accept ID|--reject ID|--accept-all|--prune)");
    eprintln!();
    eprintln!("Emits the file and its imports as LaTeX under build/.");
}

fn sole_document() -> Result<String, String> {
    let mut documents = fs::read_dir(".")
        .map_err(|error| error.to_string())?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "ntex")
        });
    let document = documents.next().ok_or("no .ntex document was found")?;
    if documents.next().is_some() {
        return Err("more than one .ntex document was found; name the file explicitly".to_owned());
    }
    Ok(document.to_string_lossy().into_owned())
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
    let prefixes = read_prefixes(Path::new(root));
    let mut table = SymbolTable::with_prefixes(prefixes);
    let mut documents = Vec::new();
    let mut pending = vec![root_id];
    let mut merged = BTreeSet::new();
    let mut import_diagnostics = Vec::new();
    let mut declared = Declared::default();

    while let Some(id) = pending.pop() {
        let name = sources
            .get(id)
            .map(|s| s.name().to_owned())
            .unwrap_or_default();
        let canonical = fs::canonicalize(&name).unwrap_or_else(|_| PathBuf::from(&name));
        if !merged.insert(canonical) {
            continue;
        }
        let document = parse(&sources, id);
        table.merge(&sources, &document);
        merge_declared(&mut declared, declared_in(&sources, id));
        let mut imports = Vec::new();
        document.walk(|node| {
            if let Node::Construct {
                kind: EntryToken::Import,
                span,
                ..
            } = node
                && let Some(path) = literal_import(&sources, id, *span)
            {
                imports.push((*span, path));
            }
        });
        for (span, path) in imports {
            match loader.load(&path, Some(id), &mut sources) {
                Ok(imported) => pending.push(imported),
                Err(IoError::NotFound { .. } | IoError::Unresolvable { .. }) => {
                    import_diagnostics.push(Diagnostic {
                        code: "NT1009",
                        entity: EntityClass::UnknownOpen,
                        name: Some(path.clone()),
                        source: id,
                        span,
                        message: format!("import path `{path}` does not resolve"),
                        related: Vec::new(),
                    });
                }
                Err(error) => return Err(error),
            }
        }
        documents.push(document);
    }

    let base = Path::new(root).parent().unwrap_or_else(|| Path::new("."));
    let bibliography = assemble(&declared, |name| fs::read(base.join(name)).ok());
    let mut diagnostics = check(&table, &bibliography);
    diagnostics.extend(check_documents(&sources, &documents, |source, name| {
        let base = sources
            .get(source)
            .and_then(|s| Path::new(s.name()).parent())
            .unwrap_or_else(|| Path::new("."));
        base.join(name).is_file()
    }));
    diagnostics.extend(import_diagnostics);
    let total_bytes: f64 = documents
        .iter()
        .filter_map(|document| sources.get(document.source()))
        .map(|source| {
            f64::from(u32::try_from(source.bytes().len()).expect("source exceeds u32 addressing"))
        })
        .sum();
    let checked_bytes: f64 = documents
        .iter()
        .filter_map(|document| {
            sources.get(document.source()).map(|source| {
                document.coverage()
                    * f64::from(
                        u32::try_from(source.bytes().len()).expect("source exceeds u32 addressing"),
                    )
            })
        })
        .sum();
    let coverage = if total_bytes == 0.0 {
        1.0
    } else {
        checked_bytes / total_bytes
    };
    Ok((sources, diagnostics, coverage, bibliography))
}

fn literal_import(
    sources: &Sources,
    source: SourceId,
    span: nextex_core::source::Span,
) -> Option<String> {
    let bytes = sources.get(source)?.slice(span)?;
    let first = bytes.iter().position(|byte| *byte == b'"')?;
    let last = bytes.iter().rposition(|byte| *byte == b'"')?;
    (last > first)
        .then(|| String::from_utf8(bytes[first + 1..last].to_vec()).ok())
        .flatten()
}

fn merge_declared(target: &mut Declared, found: Declared) {
    target.resources.extend(found.resources);
    target.inline_keys.extend(found.inline_keys);
    target.computed = target.computed.or(found.computed);
}

fn read_prefixes(input: &Path) -> PrefixMap {
    let mut dir = input.parent().unwrap_or_else(|| Path::new("."));
    loop {
        let config = dir.join("nextex.toml");
        if let Ok(text) = fs::read_to_string(config) {
            return parse_prefixes(&text).unwrap_or_default();
        }
        let Some(parent) = dir.parent() else {
            return PrefixMap::default();
        };
        if parent == dir {
            return PrefixMap::default();
        }
        dir = parent;
    }
}

fn parse_prefixes(text: &str) -> Option<PrefixMap> {
    let mut in_table = false;
    let mut found = false;
    let mut map = PrefixMap::empty();
    for raw in text.lines() {
        let line = raw.split('#').next().unwrap_or_default().trim();
        if line.starts_with('[') {
            in_table = line == "[prefixes]";
            found |= in_table;
            continue;
        }
        if !in_table || line.is_empty() {
            continue;
        }
        let (class, values) = line.split_once('=')?;
        let class = match class.trim() {
            "figure" => EntityClass::Figure,
            "table" => EntityClass::Table,
            "section" => EntityClass::Section,
            "appendix" => EntityClass::Appendix,
            "algorithm" => EntityClass::Algorithm,
            "equation" => EntityClass::Equation,
            _ => continue,
        };
        let values = values.trim().strip_prefix('[')?.strip_suffix(']')?;
        for value in values.split(',') {
            let prefix = value.trim().strip_prefix('"')?.strip_suffix('"')?;
            map.insert(prefix, class);
        }
    }
    found.then_some(map)
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

fn build(
    root: &str,
    view: RevisionView,
    loader: &FileSystem,
    sink: &mut BuildDirectory,
) -> Result<(), String> {
    let mut pending = vec![root.to_owned()];
    let mut emitted = BTreeSet::new();

    while let Some(name) = pending.pop() {
        if !emitted.insert(name.clone()) {
            continue;
        }
        let mut sources = Sources::new();
        let id = loader
            .load(&name, None, &mut sources)
            .map_err(|error| error.to_string())?;
        let document = parse(&sources, id);
        let source = sources
            .get(id)
            .expect("the loader returned an absent source");

        for node in document.iter() {
            if let Node::Malformed { kind, .. } = node
                && matches!(
                    kind,
                    EntryToken::Add | EntryToken::Del | EntryToken::Sub | EntryToken::Note
                )
            {
                return Err(format!("{} is malformed", kind.name()));
            }
        }
        validate_sidecar(source.name(), source.bytes())?;

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

        let mut output = PathBuf::from(source.name());
        if view == RevisionView::Marked {
            let stem = output.file_stem().unwrap_or_default().to_string_lossy();
            output.set_file_name(format!("{stem}.marked.tex"));
        } else {
            output.set_extension("tex");
        }
        if view == RevisionView::Final {
            let emission = emit_with_map(&sources, &document).map_err(|error| error.to_string())?;
            sink.write(&output.to_string_lossy(), &emission.bytes)
                .map_err(|error| error.to_string())?;
            output.set_extension("ntexmap");
            sink.write(&output.to_string_lossy(), &emission.map.to_json())
                .map_err(|error| error.to_string())?;
        } else {
            let mut bytes = Vec::new();
            emit_view(&sources, &document, view, &mut bytes).map_err(|error| error.to_string())?;
            sink.write(&output.to_string_lossy(), &bytes)
                .map_err(|error| error.to_string())?;
        }
    }
    Ok(())
}

fn validate_sidecar(name: &str, bytes: &[u8]) -> Result<(), String> {
    let path = Path::new(name);
    let mut sidecar_path = path.to_path_buf();
    sidecar_path.set_extension("ntexrev");
    let file_name = path.file_name().unwrap_or_default().to_string_lossy();
    let sidecar = match fs::read(&sidecar_path) {
        Ok(source) => {
            let sidecar = parse_sidecar(&source).map_err(|error| error.to_string())?;
            // A sidecar names the document it belongs to. One that names a
            // different file is paired with the wrong source, and every record
            // in it would be judged against constructs it was never about.
            //
            // Only a sidecar that exists can be mispaired. The synthesised one
            // below describes this file by construction, and comparing it
            // against itself once failed for every imported file: it was built
            // from the load path, `sections/part.ntex`, and compared against
            // the file name, `part.ntex`.
            if sidecar.document != file_name {
                return Err(format!(
                    "NT1013: {} names document '{}', not '{file_name}'",
                    sidecar_path.display(),
                    sidecar.document
                ));
            }
            sidecar
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            nextex_core::review::Sidecar {
                version: 1,
                document: file_name.into_owned(),
                revisions: Vec::new(),
            }
        }
        Err(error) => return Err(format!("{}: {error}", sidecar_path.display())),
    };
    for advisory in validate(bytes, &sidecar).map_err(|error| error.to_string())? {
        eprintln!("advisory: {}", advisory.message);
    }
    Ok(())
}

fn revise(args: &[String]) -> ExitCode {
    let Some(input) = args.first() else {
        eprintln!(
            "usage: nextex revise <file.ntex> (--accept ID|--reject ID|--accept-all|--prune)"
        );
        return ExitCode::from(2);
    };
    let path = Path::new(input);
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) => {
            eprintln!("error: {error}");
            return ExitCode::FAILURE;
        }
    };
    let mut sidecar_path = path.to_path_buf();
    sidecar_path.set_extension("ntexrev");
    let mut sidecar_bytes = fs::read(&sidecar_path).ok();
    if args.get(1).is_some_and(|option| option == "--prune") {
        return prune(&sidecar_path, sidecar_bytes.as_deref(), &bytes);
    }
    if args.get(1).is_some_and(|option| option == "--reject") && sidecar_bytes.is_none() {
        eprintln!(
            "error: rejecting requires {} so removed text remains recoverable",
            sidecar_path.display()
        );
        return ExitCode::FAILURE;
    }
    if let Some(sidecar_source) = &sidecar_bytes {
        match parse_sidecar(sidecar_source).and_then(|sidecar| validate(&bytes, &sidecar)) {
            Ok(advisories) => {
                for advisory in advisories {
                    eprintln!("advisory: {}", advisory.message);
                }
            }
            Err(error) => {
                eprintln!("error: {error}");
                return ExitCode::FAILURE;
            }
        }
    }
    let result: ResolutionResult = match args.get(1).map(String::as_str) {
        Some("--accept") => args
            .get(2)
            .ok_or_else(|| "--accept requires an identifier".to_owned())
            .and_then(|id| {
                resolve(&bytes, id, Resolution::Accept)
                    .map(|(source, removed)| {
                        (source, vec![(id.clone(), Resolution::Accept, removed)])
                    })
                    .map_err(|error| error.to_string())
            }),
        Some("--reject") => args
            .get(2)
            .ok_or_else(|| "--reject requires an identifier".to_owned())
            .and_then(|id| {
                resolve(&bytes, id, Resolution::Reject)
                    .map(|(source, removed)| {
                        (source, vec![(id.clone(), Resolution::Reject, removed)])
                    })
                    .map_err(|error| error.to_string())
            }),
        Some("--accept-all") => resolve_all(&bytes),
        _ => Err("expected --accept, --reject, or --accept-all".to_owned()),
    };
    let (rewritten, events) = match result {
        Ok(value) => value,
        Err(error) => {
            eprintln!("error: {error}");
            return ExitCode::from(2);
        }
    };
    if let Some(mut records) = sidecar_bytes.take() {
        let by = review_author();
        let at = review_timestamp();
        for (id, resolution, removed) in events {
            records = match resolve_sidecar(&records, &id, resolution, &by, &at, &removed) {
                Ok(records) => records,
                Err(error) => {
                    eprintln!("error: {error}");
                    return ExitCode::FAILURE;
                }
            };
        }
        if let Err(error) = atomic_replace(&sidecar_path, &records) {
            eprintln!("error: {error}");
            return ExitCode::FAILURE;
        }
    }
    if let Err(error) = atomic_replace(path, &rewritten) {
        eprintln!("error: {error}");
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}

fn prune(sidecar_path: &Path, records: Option<&[u8]>, source: &[u8]) -> ExitCode {
    let Some(records) = records else {
        eprintln!("error: {} does not exist", sidecar_path.display());
        return ExitCode::FAILURE;
    };
    let pruned = match prune_sidecar(records, source, &review_author(), &review_timestamp()) {
        Ok(pruned) => pruned,
        Err(error) => {
            eprintln!("error: {error}");
            return ExitCode::FAILURE;
        }
    };
    match atomic_replace(sidecar_path, &pruned) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}

fn resolve_all(bytes: &[u8]) -> Result<(Vec<u8>, Vec<ResolutionEvent>), String> {
    let mut current = bytes.to_vec();
    let mut events = Vec::new();
    loop {
        let id = nextex_core::review::revision_ids(&current)
            .into_iter()
            .next();
        let Some(id) = id else {
            return Ok((current, events));
        };
        let (source, removed) =
            resolve(&current, &id, Resolution::Accept).map_err(|error| error.to_string())?;
        current = source;
        events.push((id, Resolution::Accept, removed));
    }
}

fn atomic_replace(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or("the input has no filename")?;
    let temporary = path.with_file_name(format!(".{name}.{}.tmp", std::process::id()));
    let result = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .and_then(|mut file| {
            use std::io::Write;
            file.write_all(bytes)?;
            file.sync_all()
        })
        .and_then(|()| fs::rename(&temporary, path));
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result.map_err(|error| error.to_string())
}

fn current_timestamp() -> String {
    let seconds = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs());
    let days = i64::try_from(seconds / 86_400).unwrap_or(i64::MAX);
    let day_seconds = seconds % 86_400;
    let shifted = days + 719_468;
    let era = shifted.div_euclid(146_097);
    let day_of_era = shifted - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let mut year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_part = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_part + 2) / 5 + 1;
    let month = month_part + if month_part < 10 { 3 } else { -9 };
    year += i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        day_seconds / 3_600,
        day_seconds % 3_600 / 60,
        day_seconds % 60
    )
}

fn review_author() -> String {
    std::env::var("NEXTEX_AUTHOR")
        .or_else(|_| std::env::var("USER"))
        .unwrap_or_else(|_| "unknown".to_owned())
}

fn review_timestamp() -> String {
    std::env::var("NEXTEX_AT").unwrap_or_else(|_| current_timestamp())
}

/// Reports where a location in the emitted `.tex` came from.
///
/// TeX names a line in a file the author never wrote. Without this the author
/// reads an error against bytes they have never seen, and NextTeX carries the
/// blame for every LaTeX error in the document.
///
/// The map is rebuilt from the source rather than read back from the
/// `.ntexmap` beside the output: nothing here parses that file yet, and it
/// exists for editors and CI rather than for this path.
fn blame(mut args: impl Iterator<Item = String>) -> ExitCode {
    let (Some(input), Some(location)) = (args.next(), args.next()) else {
        eprintln!("usage: nextex blame <file.ntex> <line>:<column> [message]");
        return ExitCode::from(2);
    };
    let message = args
        .next()
        .unwrap_or_else(|| "TeX reported an error here".to_owned());

    let mut parts = location.split(':');
    let (Some(Ok(line)), Some(Ok(column))) = (
        parts.next().map(str::parse::<u32>),
        parts.next().map(str::parse::<u32>),
    ) else {
        eprintln!("error: expected a location such as 11:1, found `{location}`");
        return ExitCode::from(2);
    };

    let loader = FileSystem {
        root: PathBuf::from("."),
    };
    let mut sources = Sources::new();
    let Ok(id) = loader.load(&input, None, &mut sources) else {
        eprintln!("error: cannot read {input}");
        return ExitCode::from(2);
    };
    let document = parse(&sources, id);
    let Ok(emission) = emit_with_map(&sources, &document) else {
        eprintln!("error: {input} could not be emitted");
        return ExitCode::from(2);
    };

    let mapped = map_emitted_diagnostic(message, &emission.bytes, line, column, &emission.map);
    println!("error[TEX]: {}", mapped.message);
    let mut output = PathBuf::from(&input);
    output.set_extension("tex");
    println!("  emitted at build/{}:{line}:{column}", output.display());
    match &mapped.span {
        Some(span) => println!(
            "  corresponds to {}:{}:{}",
            span.file, span.line, span.column
        ),
        // No segment supports an answer. Saying so beats naming the nearest
        // one, which would blame the author for the emitter's own bytes.
        None => println!("  corresponds to an unmapped position"),
    }
    println!("  blame: {}", mapped.blame.as_str());
    ExitCode::SUCCESS
}

#[cfg(test)]
#[allow(clippy::items_after_test_module)]
mod tests {
    use super::*;

    fn case(name: &str, files: &[(&str, &str)]) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("nextex-issue-11-{}-{name}", std::process::id()));
        fs::create_dir_all(&dir).expect("create test project");
        for (name, contents) in files {
            fs::write(dir.join(name), contents).expect("write test input");
        }
        dir
    }

    #[test]
    fn absent_prefix_table_keeps_the_default() {
        assert_eq!(parse_prefixes("cite_commands = [\"cite\"]"), None);
    }

    #[test]
    fn configured_prefixes_replace_the_default() {
        let map = parse_prefixes("[prefixes]\nfigure = [\"image\"]\n").expect("table");
        let table = SymbolTable::with_prefixes(map);
        assert_eq!(table.demand_of("image:x"), EntityClass::Figure);
        assert_eq!(table.demand_of("fig:x"), EntityClass::UnknownOpen);
    }

    #[test]
    fn imports_merge_transitively_and_a_repeated_file_merges_once() {
        let dir = case(
            "imports",
            &[
                (
                    "main.ntex",
                    "@import(\"part.ntex\") @import(\"part.ntex\") @ref(sec:there)",
                ),
                ("part.ntex", "\\section{There} @id(sec:there)"),
            ],
        );
        let (_, diagnostics, _, _) =
            run_check(&dir.join("main.ntex").to_string_lossy()).expect("check project");
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
    }

    #[test]
    fn an_absent_literal_import_is_nt1009() {
        let dir = case(
            "missing-import",
            &[("main.ntex", "@import(\"absent.ntex\")")],
        );
        let (_, diagnostics, _, _) =
            run_check(&dir.join("main.ntex").to_string_lossy()).expect("check project");
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].code, "NT1009");
    }

    #[test]
    fn project_prefixes_replace_defaults_in_the_built_pipeline() {
        let dir = case(
            "prefixes",
            &[
                ("nextex.toml", "[prefixes]\nfigure = [\"image\"]\n"),
                (
                    "main.ntex",
                    "\\table(fig:old) { caption = {Old} } @ref(fig:old) \\table(image:new) { caption = {New} } @ref(image:new)",
                ),
            ],
        );
        let (_, diagnostics, _, _) =
            run_check(&dir.join("main.ntex").to_string_lossy()).expect("check project");
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].code, "NT1004");
        assert_eq!(diagnostics[0].name.as_deref(), Some("image:new"));
    }
}
