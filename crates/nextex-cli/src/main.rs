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
use nextex_core::review::{
    Resolution, parse_sidecar, prune_sidecar, resolve, resolve_sidecar, validate,
};
use nextex_core::scanner::EntryToken;
use nextex_core::source::{SourceId, Sources};
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
    let mut args = args.into_iter();
    let first = args.next();
    let input = if first.as_deref() == Some("build") {
        args.next()
    } else {
        first
    };
    let Some(input) = input else {
        eprintln!("usage: nextex build <file.ntex> [--original|--final|--marked]");
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

    let mut view = RevisionView::Final;
    for option in args {
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
    match build(&input, view, &loader, &mut sink) {
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

        let mut bytes = Vec::new();
        emit_view(&sources, &document, view, &mut bytes).map_err(|error| error.to_string())?;
        let mut output = PathBuf::from(source.name());
        if view == RevisionView::Marked {
            let stem = output.file_stem().unwrap_or_default().to_string_lossy();
            output.set_file_name(format!("{stem}.marked.tex"));
        } else {
            output.set_extension("tex");
        }
        sink.write(&output.to_string_lossy(), &bytes)
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}

fn validate_sidecar(name: &str, bytes: &[u8]) -> Result<(), String> {
    let path = Path::new(name);
    let mut sidecar_path = path.to_path_buf();
    sidecar_path.set_extension("ntexrev");
    let sidecar = match fs::read(&sidecar_path) {
        Ok(source) => parse_sidecar(&source).map_err(|error| error.to_string())?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            nextex_core::review::Sidecar {
                version: 1,
                document: name.to_owned(),
                revisions: Vec::new(),
            }
        }
        Err(error) => return Err(format!("{}: {error}", sidecar_path.display())),
    };
    if sidecar.document != path.file_name().unwrap_or_default().to_string_lossy() {
        return Err(format!(
            "NT1009: {} names document '{}'",
            sidecar_path.display(),
            sidecar.document
        ));
    }
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
