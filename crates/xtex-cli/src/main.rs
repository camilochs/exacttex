//! Command-line front end for the ExactTeX compiler.
//!
//! Every host assumption lives here: filesystem paths, the current directory,
//! and process exit codes. `xtex-core` has none of them, which is what lets
//! the same core compile to WebAssembly.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use xtex_core::bibliography::{Bibliography, Declared, assemble, declared_in};
use xtex_core::check::{Blame, Diagnostic, Severity, check_documents, check_with_labels, to_json};
use xtex_core::diagnostics::map_emitted_diagnostic;
use xtex_core::document::Node;
use xtex_core::editor::entity_at;
use xtex_core::io::{IoError, OutputSink, SourceLoader};
use xtex_core::review::{
    Resolution, parse_sidecar, prune_sidecar, resolve, resolve_sidecar, validate,
};
use xtex_core::scanner::EntryToken;
use xtex_core::source::{SourceId, Sources};
use xtex_core::sourcemap::emit_with_map;
use xtex_core::symbols::{EntityClass, PrefixMap, SymbolTable};
use xtex_core::{RevisionView, emit_view, parse};

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
    if first == "compile" {
        return compile_command(&args[1..]);
    }
    if first == "confidence" {
        return confidence_command(&args[1..]);
    }
    if first == "rename" {
        return rename_command(&args[1..]);
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
    eprintln!("usage: xtex <file.xtex> [--original|--final|--marked]");
    eprintln!("       xtex build <file.xtex> [--original|--final|--marked]");
    eprintln!("       xtex check [--json] [--strict-tex] <file.xtex>");
    eprintln!("       xtex blame <file.xtex> <line>:<column> [message]");
    eprintln!("       xtex revise <file.xtex> (--accept ID|--reject ID|--accept-all|--prune)");
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
    let document = documents.next().ok_or("no .xtex document was found")?;
    if documents.next().is_some() {
        return Err("more than one .xtex document was found; name the file explicitly".to_owned());
    }
    Ok(document.to_string_lossy().into_owned())
}

fn check_command(args: &[String]) -> ExitCode {
    let json = args.iter().any(|arg| arg == "--json");
    let inputs: Vec<&str> = args
        .iter()
        .filter(|arg| !arg.starts_with("--"))
        .map(String::as_str)
        .collect();
    if inputs.len() != 1 {
        eprintln!("usage: xtex check [--json] [--strict-tex] <file.xtex>");
        return ExitCode::from(2);
    }

    match run_check(inputs[0]) {
        Ok((sources, diagnostics, coverage, _bibliography)) => {
            if json {
                {
                    // One renderer, shared with the WebAssembly build, so the two
                    // cannot drift.
                    let mut json = String::new();
                    to_json(&sources, &diagnostics, coverage, &mut json);
                    println!("{json}");
                }
            } else {
                for diagnostic in &diagnostics {
                    print_human(&sources, diagnostic);
                }
                println!("coverage: {:.1}%", coverage * 100.0);
            }
            if diagnostics
                .iter()
                .all(|diagnostic| diagnostic.severity == Severity::Advisory)
            {
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
    // One inventory for the whole root, merged like the symbol table: a
    // `\label` in an imported file declares its name for the root, exactly as
    // an `@id` there does.
    let mut labels: BTreeMap<String, xtex_core::source::Span> = BTreeMap::new();
    let mut labels_unavailable = None;

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
        match xtex_core::labels::inventory(&sources, &document, id) {
            xtex_core::labels::Inventory::Complete(found) => labels.extend(found),
            // One file that went dark makes the root's inventory a subset, and
            // a subset that looks complete turns every name it missed into a
            // false "not declared".
            xtex_core::labels::Inventory::Unavailable(reason) => labels_unavailable = Some(reason),
        }
        merge_declared(&mut declared, declared_in(&sources, id));
        follow_latex_edges(
            &loader,
            id,
            &mut sources,
            &mut pending,
            &mut labels_unavailable,
        )?;
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
                    labels_unavailable = Some(xtex_core::labels::Unavailable::UnreadableEdge);
                    import_diagnostics.push(Diagnostic {
                        code: "XT1009",
                        entity: EntityClass::UnknownOpen,
                        name: Some(path.clone()),
                        source: id,
                        span,
                        message: format!("import path `{path}` does not resolve"),
                        related: Vec::new(),
                        severity: Severity::Error,
                        blame: Blame::XtexConstruct,
                    });
                }
                Err(error) => return Err(error),
            }
        }
        documents.push(document);
    }

    let base = Path::new(root).parent().unwrap_or_else(|| Path::new("."));
    let bibliography = assemble(&declared, |name| fs::read(base.join(name)).ok());
    // The author's own `\label` commands resolve `@ref` too, so annotating a
    // document one figure at a time does not report the unannotated ones as
    // missing. Merged across the root, like every other declaration.
    let inventory = root_inventory(labels, labels_unavailable);
    let mut diagnostics = check_with_labels(&table, &bibliography, &inventory);
    diagnostics.extend(bibliography_advisory(&table, &bibliography));
    diagnostics.extend(check_documents(&sources, &documents, |source, name| {
        let base = sources
            .get(source)
            .and_then(|s| Path::new(s.name()).parent())
            .unwrap_or_else(|| Path::new("."));
        base.join(name).is_file()
    }));
    diagnostics.extend(import_diagnostics);
    let coverage = root_coverage(&sources, &documents);
    Ok((sources, diagnostics, coverage, bibliography))
}

fn root_coverage(sources: &Sources, documents: &[xtex_core::document::Document]) -> f64 {
    let mut total = 0.0;
    let mut checked = 0.0;
    for document in documents {
        let Some(source) = sources.get(document.source()) else {
            continue;
        };
        let bytes =
            f64::from(u32::try_from(source.bytes().len()).expect("source exceeds u32 addressing"));
        total += bytes;
        checked += document.coverage() * bytes;
    }
    if total == 0.0 { 1.0 } else { checked / total }
}

fn follow_latex_edges(
    loader: &impl SourceLoader,
    id: SourceId,
    sources: &mut Sources,
    pending: &mut Vec<SourceId>,
    unavailable: &mut Option<xtex_core::labels::Unavailable>,
) -> Result<(), IoError> {
    let (edges, computed) = sources
        .get(id)
        .map(|source| latex_inventory_edges(source.bytes()))
        .unwrap_or_default();
    if computed {
        *unavailable = Some(xtex_core::labels::Unavailable::UnreadableEdge);
    }
    for path in edges {
        match load_latex_edge(loader, &path, id, sources) {
            Ok(included) => pending.push(included),
            Err(error @ IoError::TooLarge { .. }) => return Err(error),
            Err(_) => *unavailable = Some(xtex_core::labels::Unavailable::UnreadableEdge),
        }
    }
    Ok(())
}

fn load_latex_edge(
    loader: &impl SourceLoader,
    path: &str,
    relative_to: SourceId,
    sources: &mut Sources,
) -> Result<SourceId, IoError> {
    if Path::new(path).extension().is_some() {
        return loader.load(path, Some(relative_to), sources);
    }
    match loader.load(&format!("{path}.tex"), Some(relative_to), sources) {
        Ok(id) => Ok(id),
        Err(IoError::NotFound { .. }) => {
            loader.load(&format!("{path}.xtex"), Some(relative_to), sources)
        }
        Err(error) => Err(error),
    }
}

fn latex_inventory_edges(bytes: &[u8]) -> (Vec<String>, bool) {
    let mut edges = Vec::new();
    let mut computed = false;
    for span in xtex_core::scanner::readable_for(bytes, &["include", "input"]) {
        let region = &bytes[span.start()..span.end()];
        computed |= collect_latex_edges(region, &mut edges);
    }
    (edges, computed)
}

fn collect_latex_edges(mut region: &[u8], edges: &mut Vec<String>) -> bool {
    let mut computed = false;
    while let Some(at) = region.windows(2).position(|window| window == b"\\i") {
        region = &region[at..];
        let command_len = if region.starts_with(b"\\include") {
            b"\\include".len()
        } else if region.starts_with(b"\\input") {
            b"\\input".len()
        } else {
            region = &region[2..];
            continue;
        };
        if region
            .get(command_len)
            .is_some_and(|byte| byte.is_ascii_alphabetic() || *byte == b'@')
        {
            region = &region[command_len..];
            continue;
        }
        let rest = &region[command_len..];
        let whitespace = rest
            .iter()
            .position(|byte| !byte.is_ascii_whitespace())
            .unwrap_or(rest.len());
        let rest = &rest[whitespace..];
        let Some(body) = rest.strip_prefix(b"{") else {
            computed = true;
            region = rest;
            continue;
        };
        let Some(close) = body.iter().position(|byte| *byte == b'}') else {
            return true;
        };
        let path = &body[..close];
        if path
            .iter()
            .any(|byte| matches!(byte, b'\\' | b'{' | b'}' | b'#'))
        {
            computed = true;
        } else if let Ok(path) = std::str::from_utf8(path) {
            let path = path.trim();
            if path.is_empty() {
                computed = true;
            } else {
                edges.push(path.to_owned());
            }
        } else {
            computed = true;
        }
        region = &body[close + 1..];
    }
    computed
}

fn literal_import(
    sources: &Sources,
    source: SourceId,
    span: xtex_core::source::Span,
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
        let config = dir.join("xtex.toml");
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

fn bibliography_advisory(table: &SymbolTable, bibliography: &Bibliography) -> Option<Diagnostic> {
    let Bibliography::Unavailable(reason) = bibliography else {
        return None;
    };
    let (_, citation) = table.citations().next()?;
    Some(Diagnostic {
        code: "XT2001",
        entity: EntityClass::Citation,
        name: None,
        source: citation.payload.source,
        span: citation.payload.span,
        message: format!("citation checking unavailable: {}", reason.reason()),
        related: Vec::new(),
        severity: Severity::Advisory,
        blame: Blame::Unresolved,
    })
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
            output.set_extension("xtexmap");
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
    sidecar_path.set_extension("xtexrev");
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
            // from the load path, `sections/part.xtex`, and compared against
            // the file name, `part.xtex`.
            if sidecar.document != file_name {
                return Err(format!(
                    "XT1013: {} names document '{}', not '{file_name}'",
                    sidecar_path.display(),
                    sidecar.document
                ));
            }
            sidecar
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => xtex_core::review::Sidecar {
            version: 1,
            document: file_name.into_owned(),
            revisions: Vec::new(),
        },
        Err(error) => return Err(format!("{}: {error}", sidecar_path.display())),
    };
    for advisory in validate(bytes, &sidecar).map_err(|error| error.to_string())? {
        eprintln!("advisory: {}", advisory.message);
    }
    Ok(())
}

fn revise(args: &[String]) -> ExitCode {
    let Some(input) = args.first() else {
        eprintln!("usage: xtex revise <file.xtex> (--accept ID|--reject ID|--accept-all|--prune)");
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
    sidecar_path.set_extension("xtexrev");
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
        let id = xtex_core::review::revision_ids(&current).into_iter().next();
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
    std::env::var("XTEX_AUTHOR")
        .or_else(|_| std::env::var("USER"))
        .unwrap_or_else(|_| "unknown".to_owned())
}

fn review_timestamp() -> String {
    std::env::var("XTEX_AT").unwrap_or_else(|_| current_timestamp())
}

/// Reports where a location in the emitted `.tex` came from.
///
/// TeX names a line in a file the author never wrote. Without this the author
/// reads an error against bytes they have never seen, and ExactTeX carries the
/// blame for every LaTeX error in the document.
///
/// The map is rebuilt from the source rather than read back from the
/// `.xtexmap` beside the output: nothing here parses that file yet, and it
/// exists for editors and CI rather than for this path.
fn blame(mut args: impl Iterator<Item = String>) -> ExitCode {
    let (Some(input), Some(location)) = (args.next(), args.next()) else {
        eprintln!("usage: xtex blame <file.xtex> <line>:<column> [message]");
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
mod cause_tests {
    use super::cause_at;

    #[test]
    fn a_control_word_is_named_and_an_environment_is_named_with_it() {
        // These are what a corpus groups by. `\\begin` alone would put
        // `lstlisting` and `verbatim` in one bucket, and they are not one
        // problem.
        assert_eq!(cause_at(b"\\verb+unterminated", 0), "\\verb");
        assert_eq!(cause_at(b"\\catcode`\\@=11", 0), "\\catcode");
        assert_eq!(
            cause_at(b"\\begin{lstlisting}\ncode", 0),
            "\\begin{lstlisting}"
        );
        assert_eq!(cause_at(b"\\begin{verbatim}\nx", 0), "\\begin{verbatim}");
    }

    #[test]
    fn without_a_control_word_the_bytes_are_shown_rather_than_a_cause_invented() {
        // Naming a cause we cannot see would be worse than showing the bytes,
        // because a corpus tally of invented causes reads exactly like a tally
        // of real ones.
        assert_eq!(cause_at(b"plain text here", 0), "bytes `plain text here`");
        assert_eq!(cause_at(b"\\  spaced", 0), "bytes `\\  spaced`");
    }

    #[test]
    fn an_offset_at_or_past_the_end_does_not_panic() {
        assert_eq!(cause_at(b"abc", 3), "bytes ``");
        assert_eq!(cause_at(b"", 99), "bytes ``");
    }
}

#[cfg(test)]
#[allow(clippy::items_after_test_module)]
mod tests {
    use super::*;

    fn case(name: &str, files: &[(&str, &str)]) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("xtex-issue-11-{}-{name}", std::process::id()));
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

    /// The exit criterion of #17, over a real multi-file project.
    #[test]
    fn renaming_a_root_leaves_no_broken_reference_and_no_rewritten_prose() {
        let dir = case(
            "rename",
            &[
                (
                    "main.xtex",
                    "@import(\"part.xtex\")\n\
                     As shown in @ref(fig:model), and again in @ref(fig:model).\n\
                     The author wrote \\ref{fig:model} by hand, and \\verb|fig:model| too.\n\
                     We discuss fig:model in this sentence.\n",
                ),
                (
                    "part.xtex",
                    "\\figure(fig:model) { src = \"p.pdf\" caption = {The fig:model itself} }\n\
                     See @ref(fig:model) from the imported file.\n",
                ),
            ],
        );
        fs::write(dir.join("p.pdf"), b"").expect("an image to resolve");
        let before = fs::read_to_string(dir.join("main.xtex")).expect("read");

        let root = dir.join("main.xtex").to_string_lossy().into_owned();
        let code = rename_command(&[root.clone(), "fig:model".to_owned(), "fig:arch".to_owned()]);
        assert_eq!(code, ExitCode::SUCCESS);

        let main = fs::read_to_string(dir.join("main.xtex")).expect("read");
        let part = fs::read_to_string(dir.join("part.xtex")).expect("read");

        // 1. No explicit reference is broken.
        let (_, diagnostics, _, _) = run_check(&root).expect("check the renamed project");
        assert!(diagnostics.is_empty(), "{diagnostics:?}");

        // 2. No structurally resolved occurrence of the old name survives.
        assert!(!main.contains("@ref(fig:model)"), "{main}");
        assert!(!part.contains("@ref(fig:model)"), "{part}");
        assert!(!part.contains("\\figure(fig:model)"), "{part}");

        // 3. Every occurrence in opaque text is byte-identical.
        for opaque in [
            "\\ref{fig:model}",
            "\\verb|fig:model|",
            "We discuss fig:model in this sentence.",
        ] {
            assert!(main.contains(opaque), "opaque text was rewritten: {main}");
        }
        assert!(
            part.contains("caption = {The fig:arch itself}")
                || part.contains("{The fig:model itself}"),
            "the caption is content either way: {part}"
        );
        assert_ne!(before, main, "something changed");
    }

    #[test]
    fn imports_merge_transitively_and_a_repeated_file_merges_once() {
        let dir = case(
            "imports",
            &[
                (
                    "main.xtex",
                    "@import(\"part.xtex\") @import(\"part.xtex\") @ref(sec:there)",
                ),
                ("part.xtex", "\\section{There} @id(sec:there)"),
            ],
        );
        let (_, diagnostics, _, _) =
            run_check(&dir.join("main.xtex").to_string_lossy()).expect("check project");
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
    }

    #[test]
    fn an_absent_literal_import_is_nt1009() {
        let dir = case(
            "missing-import",
            &[("main.xtex", "@import(\"absent.xtex\")")],
        );
        let (_, diagnostics, _, _) =
            run_check(&dir.join("main.xtex").to_string_lossy()).expect("check project");
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].code, "XT1009");
    }

    #[test]
    fn project_prefixes_replace_defaults_in_the_built_pipeline() {
        let dir = case(
            "prefixes",
            &[
                ("xtex.toml", "[prefixes]\nfigure = [\"image\"]\n"),
                (
                    "main.xtex",
                    "\\table(fig:old) { caption = {Old} } @ref(fig:old) \\table(image:new) { caption = {New} } @ref(image:new)",
                ),
            ],
        );
        let (_, diagnostics, _, _) =
            run_check(&dir.join("main.xtex").to_string_lossy()).expect("check project");
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].code, "XT1004");
        assert_eq!(diagnostics[0].name.as_deref(), Some("image:new"));
    }
}

/// Renames an identifier across a document root.
///
/// The root, not the file: an identifier is scoped to the root file plus
/// everything it imports, so renaming one file at a time would leave the rest
/// pointing at a name that no longer exists.
///
/// What it will not rewrite, it reports. A `\label{fig:plot}` the author wrote
/// is transported LaTeX and is never touched — and an author who is not told
/// about it has a document that used to work.
fn rename_command(args: &[String]) -> ExitCode {
    let [input, from, to] = args else {
        eprintln!("usage: xtex rename <file.xtex> <old> <new>");
        return ExitCode::from(2);
    };

    let loader = FileSystem {
        root: PathBuf::from("."),
    };
    let mut sources = Sources::new();
    let mut documents = Vec::new();
    let mut names = Vec::new();
    let mut pending = vec![(input.clone(), None)];
    let mut seen = BTreeSet::new();
    while let Some((name, parent)) = pending.pop() {
        if !seen.insert(name.clone()) {
            continue;
        }
        let id = match loader.load(&name, parent, &mut sources) {
            Ok(id) => id,
            Err(error) => {
                eprintln!("error: {error}");
                return ExitCode::from(2);
            }
        };
        let document = parse(&sources, id);
        let mut imports = Vec::new();
        document.walk(|node| {
            if let Node::Construct {
                kind: EntryToken::Import,
                span,
                ..
            } = node
                && let Some(path) = literal_import(&sources, id, *span)
            {
                imports.push(path);
            }
        });
        for path in imports {
            pending.push((path, Some(id)));
        }
        // The source's own name, not the one it was asked for. An import is
        // requested as `part.xtex` and stored as the path that actually
        // resolved, and writing to the request would put the file wherever the
        // process happens to be running.
        let resolved = sources
            .get(id)
            .map_or_else(|| name.clone(), |source| source.name().to_owned());
        names.push((id, resolved));
        documents.push(document);
    }

    let plan = xtex_core::rename::plan(&sources, &documents, from, to);
    if plan.is_empty() {
        eprintln!("nothing named `{from}` is declared or referenced in this root");
        return ExitCode::from(1);
    }

    for (id, name) in &names {
        let count = plan.edits.iter().filter(|edit| edit.source == *id).count();
        if count == 0 {
            continue;
        }
        let Some(bytes) = sources.get(*id).map(|source| source.bytes().to_vec()) else {
            continue;
        };
        let updated = xtex_core::rename::apply(&bytes, *id, &plan);
        if let Err(error) = atomic_replace(Path::new(name), &updated) {
            eprintln!("error: {name}: {error}");
            return ExitCode::from(2);
        }
        println!("{name}: {count} renamed");
    }

    for found in &plan.untouched {
        let Some(source) = sources.get(found.source) else {
            continue;
        };
        let (line, column) = offset_line_column(source.bytes(), found.span.start());
        println!(
            "  left alone: {}:{line}:{column} — transported LaTeX is never rewritten",
            source.name()
        );
    }
    ExitCode::SUCCESS
}

/// One-based line and column of a byte offset.
// Clippy suggests the `bytecount` crate. `docs/decisions/0005` is why we do not
// take it, and the counting here happens once per reported occurrence.
#[allow(clippy::naive_bytecount)]
fn offset_line_column(bytes: &[u8], offset: usize) -> (usize, usize) {
    let before = &bytes[..offset.min(bytes.len())];
    let line = before.iter().filter(|byte| **byte == b'\n').count() + 1;
    let start = before
        .iter()
        .rposition(|byte| *byte == b'\n')
        .map_or(0, |at| at + 1);
    (line, offset - start + 1)
}

/// Prints one diagnostic the way a person reads it.
fn print_human(sources: &Sources, diagnostic: &Diagnostic) {
    let (file, line, column) = location(sources, diagnostic.source, diagnostic.span);
    match diagnostic.severity {
        Severity::Error => println!("error[{}]: {}", diagnostic.code, diagnostic.message),
        Severity::Advisory => println!("advisory: {}", diagnostic.message),
    }
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
    println!("  blame: {}", diagnostic.blame.as_str());
}

/// File, one-based line and one-based column of a span.
fn location(
    sources: &Sources,
    source: SourceId,
    span: xtex_core::source::Span,
) -> (String, usize, usize) {
    let Some(source) = sources.get(source) else {
        return ("<unresolved>".to_owned(), 1, 1);
    };
    let bytes = source.bytes();
    let before = &bytes[..span.start().min(bytes.len())];
    // Clippy suggests `bytecount`; `docs/decisions/0005` is why we do not.
    #[allow(clippy::naive_bytecount)]
    let line = before.iter().filter(|byte| **byte == b'\n').count() + 1;
    let column = before
        .iter()
        .rposition(|byte| *byte == b'\n')
        .map_or(before.len() + 1, |at| before.len() - at);
    (source.name().to_owned(), line, column)
}

/// Reports how much of a file the parser recognises before giving up.
///
/// One line, meant for `tests/corpus/measure.py` rather than for a person:
/// the fraction of the file available before `OpaqueToEof`, or `none` when
/// recognition never stops.
fn confidence_command(args: &[String]) -> ExitCode {
    let Some(input) = args.first() else {
        eprintln!("usage: xtex confidence <file.tex>");
        return ExitCode::from(2);
    };
    let loader = FileSystem {
        root: PathBuf::from("."),
    };
    let mut sources = Sources::new();
    let Ok(id) = loader.load(input, None, &mut sources) else {
        eprintln!("error: cannot read {input}");
        return ExitCode::from(2);
    };
    let document = parse(&sources, id);
    let Some(source) = sources.get(id) else {
        return ExitCode::from(2);
    };
    let total = source.bytes().len();

    match document.quarantine_position() {
        // A file with no bytes is available in full rather than not at all.
        Some(at) if total > 0 => {
            #[allow(clippy::cast_precision_loss)]
            let fraction = at as f64 / total as f64;
            println!("quarantine: {fraction:.6}");
            println!("cause: {}", cause_at(source.bytes(), at));
        }
        _ => println!("quarantine: none"),
    }
    println!("bytes: {total}");
    println!("coverage: {:.6}", document.coverage());
    ExitCode::SUCCESS
}

/// What is at the byte where recognition stopped.
///
/// A quarantine figure says how much of a file was reachable. It does not say
/// what to fix, and over a corpus that is the difference between a number and
/// a list of work. Reported as the control word where there is one, because
/// those group — a hundred files stopped by `\catcode` are one problem — and
/// as a short excerpt otherwise, because inventing a cause is worse than
/// showing the bytes.
fn cause_at(bytes: &[u8], at: usize) -> String {
    let rest = &bytes[at.min(bytes.len())..];
    if rest.first() == Some(&b'\\') {
        let name: Vec<u8> = rest[1..]
            .iter()
            .take_while(|byte| byte.is_ascii_alphabetic())
            .copied()
            .collect();
        if !name.is_empty() {
            let name = String::from_utf8_lossy(&name);
            // `\begin{lstlisting}` and `\begin{verbatim}` are different
            // problems and collapsing them into `\begin` would hide that.
            if name == "begin" {
                if let Some(open) = rest.get(6) {
                    if *open == b'{' {
                        let environment: Vec<u8> = rest[7..]
                            .iter()
                            .take_while(|byte| **byte != b'}')
                            .copied()
                            .collect();
                        return format!("\\begin{{{}}}", String::from_utf8_lossy(&environment));
                    }
                }
            }
            return format!("\\{name}");
        }
    }
    let excerpt: String = String::from_utf8_lossy(&rest[..rest.len().min(40)])
        .chars()
        .map(|c| if c.is_whitespace() { ' ' } else { c })
        .collect();
    format!("bytes `{}`", excerpt.trim())
}

/// Emits, runs the TeX engine, and reports what it said against the source.
///
/// The engine reports against a file the author has never seen. Everything
/// here exists to close that gap: the message is the engine's, unchanged, and
/// the location beside it is the author's.
fn compile_command(args: &[String]) -> ExitCode {
    let Some(input) = args.first() else {
        eprintln!("usage: xtex compile <file.xtex>");
        return ExitCode::from(2);
    };

    let loader = FileSystem {
        root: PathBuf::from("."),
    };
    let mut sources = Sources::new();
    let Ok(id) = loader.load(input, None, &mut sources) else {
        eprintln!("error: cannot read {input}");
        return ExitCode::from(2);
    };
    let document = parse(&sources, id);
    let mut table = SymbolTable::new();
    table.merge(&sources, &document);
    let Ok(emission) = emit_with_map(&sources, &document) else {
        eprintln!("error: {input} could not be emitted");
        return ExitCode::from(2);
    };

    let mut emitted = PathBuf::from("build").join(input);
    emitted.set_extension("tex");
    if let Some(dir) = emitted.parent()
        && let Err(error) = fs::create_dir_all(dir)
    {
        eprintln!("error: {}: {error}", dir.display());
        return ExitCode::from(2);
    }
    if let Err(error) = fs::write(&emitted, &emission.bytes) {
        eprintln!("error: {}: {error}", emitted.display());
        return ExitCode::from(2);
    }

    // The engine is named by the project rather than assumed. `AGENTS.md` §4
    // forbids proposing a dependency from memory, and an engine is one.
    let engine = tex_engine();
    let Ok(run) = Command::new(&engine)
        .args(["-X", "compile", "--keep-logs", "--outfmt", "pdf"])
        .arg(emitted.file_name().unwrap_or_default())
        .current_dir(emitted.parent().unwrap_or(Path::new(".")))
        .output()
    else {
        eprintln!("error: cannot run `{engine}`. Name another in xtex.toml under [tex] command.");
        return ExitCode::from(2);
    };

    let output = String::from_utf8_lossy(&run.stderr);
    let mut records = xtex_core::texlog::parse(&output);

    // The raw log carries records stderr never shows — `Float too large for
    // page` is one, checked against a live run. Reading stderr alone would
    // silently miss a whole class of failure.
    let mut log = emitted.clone();
    log.set_extension("log");
    if let Ok(text) = fs::read_to_string(&log) {
        let name = emitted
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        for record in xtex_core::texlog::parse_log(&text, &name) {
            // A typesetting failure supersedes the plainer stderr line for the
            // same message, so the restated form is the one an author reads.
            let xtex_core::texlog::Record::Typeset { message, .. } = &record else {
                continue;
            };
            records.retain(|existing| {
                !matches!(
                    existing,
                    xtex_core::texlog::Record::Located { message: other, .. } if other == message
                )
            });
            if !records.contains(&record) {
                records.push(record);
            }
        }
    }
    let mut reported = 0usize;
    for record in &records {
        reported += usize::from(report_record(
            record, &emission, &emitted, &sources, &document, &table,
        ));
    }

    if reported == 0 && run.status.success() {
        println!("{}: compiled with nothing to report", emitted.display());
    }
    if run.status.success() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

/// The engine this project uses.
///
/// `tectonic` by default because it needs no installed distribution, and
/// replaceable in `xtex.toml` so the choice is the project's rather than ours.
fn tex_engine() -> String {
    fs::read_to_string("xtex.toml")
        .ok()
        .and_then(|text| {
            text.lines()
                .find_map(|line| line.trim().strip_prefix("command = ").map(str::to_owned))
        })
        .map_or_else(
            || "tectonic".to_owned(),
            |value| value.trim().trim_matches('"').to_owned(),
        )
}

/// Prints one engine record, and says whether it had a location to print.
///
/// Split out of `compile_command` because that function was doing three jobs;
/// this is the one an author actually reads.
fn report_record(
    record: &xtex_core::texlog::Record,
    emission: &xtex_core::sourcemap::MappedEmission,
    emitted: &Path,
    sources: &Sources,
    document: &xtex_core::document::Document,
    table: &SymbolTable,
) -> bool {
    use xtex_core::texlog::{Record, Severity};

    let (label, line, message, visual) = match record {
        Record::Located {
            severity,
            line,
            message,
            ..
        } => (
            match severity {
                Severity::Error => "error[TEX]",
                Severity::Warning => "warning[TEX]",
            },
            *line,
            message,
            None,
        ),
        Record::Typeset {
            visual,
            line,
            message,
            ..
        } => ("warning[TEX]", *line, message, Some(*visual)),
        // Kept and printed unchanged. An engine's output is evidence, and a
        // line we cannot place is still a line it said.
        Record::Unrecognised(line) => {
            println!("{line}");
            return false;
        }
    };

    let mapped = map_emitted_diagnostic(message.clone(), &emission.bytes, line, 1, &emission.map);

    // A typesetting failure is restated in the author's terms only when a map
    // segment and a declared entity both supply the evidence. Where either is
    // missing the engine's own sentence stands, because "something overflows"
    // is worse than a message that at least locates a box.
    let named = visual.and_then(|visual| {
        let span = mapped.span.as_ref()?;
        let (name, class) = entity_at(sources, document, table, span.offset as usize)?;
        Some((visual, name, class))
    });

    match named {
        Some((visual, name, class)) => {
            println!("{label}: {} `{name}` {}", class.name(), visual.name());
            println!("  TeX said: {message}");
        }
        None => println!("{label}: {}", mapped.message),
    }
    println!("  emitted at {}:{line}", emitted.display());
    match &mapped.span {
        Some(span) => println!(
            "  corresponds to {}:{}:{}",
            span.file, span.line, span.column
        ),
        None => println!("  corresponds to an unmapped position"),
    }
    println!("  blame: {}", mapped.blame.as_str());
    true
}

/// One inventory for a whole document root.
///
/// Complete or unavailable, never partial: a subset that looks complete turns
/// every name it missed into a false "not declared", which is the failure this
/// whole inventory exists to prevent.
fn root_inventory(
    labels: BTreeMap<String, xtex_core::source::Span>,
    unavailable: Option<xtex_core::labels::Unavailable>,
) -> xtex_core::labels::Inventory {
    match unavailable {
        Some(reason) => xtex_core::labels::Inventory::Unavailable(reason),
        None => xtex_core::labels::Inventory::Complete(labels),
    }
}

#[cfg(test)]
mod bibliography_advisory_tests {
    use super::*;
    use xtex_core::bibliography::Unavailable;

    fn table(text: &str) -> (Sources, SymbolTable) {
        let mut sources = Sources::new();
        let id = sources.add("main.xtex", text.as_bytes().to_vec());
        let document = parse(&sources, id);
        let mut table = SymbolTable::new();
        table.merge(&sources, &document);
        (sources, table)
    }

    #[test]
    fn unreadable_bibliography_with_a_citation_is_an_advisory_in_json() {
        let (sources, table) = table("See @cite(knuth1984).");
        let bibliography = Bibliography::Unavailable(Unavailable::Unreadable {
            name: "refs.bib".to_owned(),
        });
        let advisory = bibliography_advisory(&table, &bibliography)
            .expect("the explicit citation requests bibliography checking");

        assert_eq!(advisory.severity, Severity::Advisory);
        assert_eq!(advisory.blame, Blame::Unresolved);
        assert_eq!(advisory.name, None, "the advisory is not about a key");
        let mut json = String::new();
        to_json(&sources, &[advisory], 1.0, &mut json);
        assert!(json.contains("\"severity\":\"advisory\""), "{json}");
        assert!(
            json.contains("citation checking unavailable: `refs.bib` could not be read"),
            "{json}"
        );
    }

    #[test]
    fn no_advisory_message_carries_a_derived_debug_spelling() {
        // The reader of this line is the author, not the compiler. Field names
        // and byte offsets belong in neither output form.
        let (_, table) = table("See @cite(knuth1984).");
        for reason in [
            Unavailable::NoneDeclared,
            Unavailable::ComputedPath {
                span: xtex_core::source::Span::new(0, 1),
            },
            Unavailable::Unreadable {
                name: "refs.bib".to_owned(),
            },
            Unavailable::UnparsableEntry {
                name: "refs.bib".to_owned(),
                detail: "a value opened at line 3 is never closed".to_owned(),
            },
        ] {
            let message = bibliography_advisory(&table, &Bibliography::Unavailable(reason.clone()))
                .expect("the explicit citation requests bibliography checking")
                .message;
            for tell in ["{", "}", "Span", "start:", "name:"] {
                assert!(
                    !message.contains(tell),
                    "`{tell}` leaked into `{message}` from `{reason:?}`"
                );
            }
        }
    }

    #[test]
    fn unreadable_bibliography_without_a_citation_says_nothing() {
        // Declarations, references and a plain LaTeX `\cite` — every kind of
        // construct except the one that asks about a bibliography. Anchoring on
        // any of them would report an unreadable file at a document that never
        // asked, which is the invariant in `AGENTS.md` §4.
        let (_, table) = table(
            "@id(intro) here, @ref(intro) and @ref(absent).\n\\cite{knuth1984}\n\\bibliography{refs}",
        );
        let bibliography = Bibliography::Unavailable(Unavailable::Unreadable {
            name: "refs.bib".to_owned(),
        });

        assert!(bibliography_advisory(&table, &bibliography).is_none());
    }

    #[test]
    fn complete_bibliography_says_nothing() {
        let (_, table) = table("See @cite(knuth1984).");
        let bibliography = Bibliography::Complete(BTreeSet::from(["knuth1984".to_owned()]));

        assert!(bibliography_advisory(&table, &bibliography).is_none());
    }
}
