//! `xtex-verify <root.tex>` — verify a project's external claims and
//! write the dated record beside the root.
//!
//! This binary is the network's only door. The compiler never calls it;
//! a person or an editor does, and the record it leaves is what
//! `xtex check` reads offline.

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Duration;

use xtex_core::io::{IoError, SourceLoader};
use xtex_core::source::{SourceId, Sources};

mod filesystem {
    //! A minimal filesystem loader, mirroring the CLI's: root-relative
    //! logical names, no absolute paths in what the core sees. To be
    //! unified with the CLI's when the verifier graduates into it.
    use super::{IoError, Path, PathBuf, SourceId, SourceLoader, Sources};

    pub struct FileSystem {
        pub root: PathBuf,
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
            let bytes = std::fs::read(&path).map_err(|error| match error.kind() {
                std::io::ErrorKind::NotFound => IoError::NotFound {
                    name: path.display().to_string(),
                },
                _ => IoError::Unreadable {
                    name: path.display().to_string(),
                    detail: error.to_string(),
                },
            })?;
            let logical = match base.as_deref().and_then(|b| Path::new(b).parent()) {
                Some(dir) => dir.join(name).to_string_lossy().into_owned(),
                None => name.to_owned(),
            };
            Ok(sources.add(logical, bytes))
        }

        fn read_aux(&self, beside: &str, name: &str) -> Option<Vec<u8>> {
            let base = Path::new(beside).parent()?;
            std::fs::read(self.root.join(base).join(name)).ok()
        }

        fn file_exists(&self, relative_to: &str, name: &str) -> bool {
            Path::new(relative_to)
                .parent()
                .is_some_and(|base| self.root.join(base).join(name).is_file())
        }
    }
}

fn now_utc() -> String {
    // Civil date from the system clock, by hand: this crate carries no
    // datetime dependency either.
    let seconds = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let days = i64::try_from(seconds / 86_400).unwrap_or(0);
    let rem = seconds % 86_400;
    // Howard Hinnant's civil-from-days.
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if month <= 2 { year + 1 } else { year };
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        rem / 3600,
        (rem % 3600) / 60,
        rem % 60
    )
}

/// `xtex-verify materialize <doi>` — the constructive half: a BibTeX
/// entry transcribed from the DOI's live record, printed with its dated
/// provenance comment. Nothing is composed from memory; a DOI the source
/// does not know is an error, never a guessed entry.
fn materialize(args: &[String]) -> ExitCode {
    let mut doi = None;
    let mut key = None;
    let mut now = None;
    for argument in args {
        if let Some(value) = argument.strip_prefix("--key=") {
            key = Some(value.to_owned());
        } else if let Some(value) = argument.strip_prefix("--now=") {
            now = Some(value.to_owned());
        } else if argument.starts_with("--") {
            eprintln!("usage: xtex-verify materialize <doi> [--key=name]");
            return ExitCode::from(2);
        } else {
            doi = Some(argument.clone());
        }
    }
    let Some(doi) = doi else {
        eprintln!("usage: xtex-verify materialize <doi> [--key=name]");
        return ExitCode::from(2);
    };
    match xtex_verify::materialize::entry_from_doi(
        &xtex_verify::transport::Http,
        "xtex-verify (https://github.com/camilochs/exacttex)",
        Duration::from_secs(10),
        &doi,
        key.as_deref(),
        &now.unwrap_or_else(now_utc),
    ) {
        Ok(entry) => {
            println!("{}", entry.bibtex);
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::from(2)
        }
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().is_some_and(|first| first == "materialize") {
        return materialize(&args[1..]);
    }
    let mut root_arg = None;
    let mut max_age_days = 30i64;
    let mut mailto = String::new();
    let mut now = None;
    for argument in &args {
        if let Some(value) = argument.strip_prefix("--max-age=") {
            let Some(days) = value.strip_suffix('d').and_then(|d| d.parse().ok()) else {
                eprintln!("error: --max-age wants a shape like 30d");
                return ExitCode::from(2);
            };
            max_age_days = days;
        } else if let Some(value) = argument.strip_prefix("--mailto=") {
            value.clone_into(&mut mailto);
        } else if let Some(value) = argument.strip_prefix("--now=") {
            now = Some(value.to_owned());
        } else if argument.starts_with("--") {
            eprintln!("usage: xtex-verify [--max-age=30d] [--mailto=you@example.org] <root.tex>");
            return ExitCode::from(2);
        } else {
            root_arg = Some(argument.clone());
        }
    }
    let Some(root) = root_arg else {
        eprintln!("usage: xtex-verify [--max-age=30d] [--mailto=you@example.org] <root.tex>");
        return ExitCode::from(2);
    };

    let loader = filesystem::FileSystem {
        root: PathBuf::from("."),
    };
    let analysed = match xtex_core::project::analyse(&loader, &root) {
        Ok(analysed) => analysed,
        Err(error) => {
            eprintln!("error: {error}");
            return ExitCode::from(2);
        }
    };
    let xtex_core::project::Analysed {
        mut sources, names, ..
    } = analysed;
    let ids: Vec<_> = names.iter().map(|(id, _)| *id).collect();
    let Some(&root_id) = ids.first() else {
        eprintln!("error: nothing loaded");
        return ExitCode::from(2);
    };
    let claims = xtex_core::claims::collect(&mut sources, &loader, root_id, &ids);

    let record_path = Path::new(&root)
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(".xtexverified");
    let previous = std::fs::read(&record_path).ok();

    let user_agent = if mailto.is_empty() {
        "xtex-verify (https://github.com/camilochs/exacttex)".to_owned()
    } else {
        format!("xtex-verify (https://github.com/camilochs/exacttex; mailto:{mailto})")
    };
    let persist_path = record_path.clone();
    let mut persist = move |record: &xtex_core::verification::VerificationRecord| {
        let _ = std::fs::write(&persist_path, xtex_verify::run::render(record));
    };
    let mut progress = |line: &str| eprintln!("{line}");
    let mut run = xtex_verify::run::Run {
        transport: &xtex_verify::transport::Http,
        user_agent,
        now: now.unwrap_or_else(now_utc),
        max_age_days,
        timeout: Duration::from_secs(10),
        persist: &mut persist,
        progress: &mut progress,
    };
    let (record, _metrics) = xtex_verify::run::verify(&mut run, &claims, previous.as_deref());
    let _ = std::fs::write(&record_path, xtex_verify::run::render(&record));
    eprintln!(
        "record: {} claims → {}",
        record.claims.len(),
        record_path.display()
    );
    ExitCode::SUCCESS
}
