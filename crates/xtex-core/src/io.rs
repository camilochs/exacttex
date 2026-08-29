//! The boundary between the compiler and its host.
//!
//! Everything the core needs from outside itself arrives through these traits:
//! reading a source, resolving one name against another, and writing output.
//! The core contains no filesystem paths, no current directory, and no process
//! spawning, so the same crate serves a native binary and a WebAssembly build
//! without a second implementation.
//!
//! [`Memory`] is the in-process implementation used by tests. A filesystem
//! implementation lives in the CLI crate, where host assumptions belong.

use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;

use crate::source::{SourceId, Sources};

/// Why a host operation could not be completed.
///
/// Deliberately coarse: the core does not model host error taxonomies, it only
/// distinguishes the cases it must behave differently for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IoError {
    /// Nothing was found under that name.
    NotFound {
        /// The name that was requested, as the caller wrote it.
        name: String,
    },
    /// Something was found but could not be read.
    Unreadable {
        /// The name that was requested.
        name: String,
        /// What the host reported.
        detail: String,
    },
    /// The name could not be resolved to a location the loader accepts.
    Unresolvable {
        /// The name that was requested.
        name: String,
        /// Why resolution failed.
        detail: String,
    },
    /// The content exceeds a configured limit.
    TooLarge {
        /// The name that was requested.
        name: String,
        /// Size in bytes that was refused.
        len: usize,
    },
    /// Output could not be written.
    WriteFailed {
        /// The name that was being written.
        name: String,
        /// What the host reported.
        detail: String,
    },
}

impl fmt::Display for IoError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotFound { name } => write!(f, "not found: {name}"),
            Self::Unreadable { name, detail } => write!(f, "cannot read {name}: {detail}"),
            Self::Unresolvable { name, detail } => write!(f, "cannot resolve {name}: {detail}"),
            Self::TooLarge { name, len } => write!(f, "{name} is too large: {len} bytes"),
            Self::WriteFailed { name, detail } => write!(f, "cannot write {name}: {detail}"),
        }
    }
}

impl Error for IoError {}

/// Supplies source bytes to the compiler.
///
/// Implementations decide what a name means. The core passes names through
/// unchanged and never inspects them, so a name may be a project-relative path,
/// an editor URI, or a fixture label.
pub trait SourceLoader {
    /// Loads `name`, interning it into `sources` and returning its handle.
    ///
    /// When `relative_to` is `Some`, the name is resolved against that source
    /// rather than against a project root. This is what makes a path inside an
    /// imported file resolve from that file's location.
    ///
    /// # Errors
    ///
    /// Returns [`IoError`] when the name cannot be resolved, found, or read.
    fn load(
        &self,
        name: &str,
        relative_to: Option<SourceId>,
        sources: &mut Sources,
    ) -> Result<SourceId, IoError>;
}

/// Receives the bytes the compiler emits.
pub trait OutputSink {
    /// Writes `bytes` under `name`, replacing anything already there.
    ///
    /// # Errors
    ///
    /// Returns [`IoError::WriteFailed`] when the host refuses the write.
    fn write(&mut self, name: &str, bytes: &[u8]) -> Result<(), IoError>;
}

/// In-process [`SourceLoader`] and [`OutputSink`], backed by maps.
///
/// Used by tests and by the WebAssembly build, where the caller supplies
/// buffers rather than a filesystem. Name resolution is deliberately literal:
/// a name relative to another source is the other source's name up to its last
/// `/`, joined with the requested name. No `..` handling, no normalization, no
/// host semantics.
#[derive(Debug, Default, Clone)]
pub struct Memory {
    inputs: BTreeMap<String, Vec<u8>>,
    outputs: BTreeMap<String, Vec<u8>>,
}

impl Memory {
    /// Creates an empty store.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds an input under `name`.
    #[must_use]
    pub fn with_input(mut self, name: impl Into<String>, bytes: impl Into<Vec<u8>>) -> Self {
        self.inputs.insert(name.into(), bytes.into());
        self
    }

    /// Bytes written under `name`, if any.
    #[must_use]
    pub fn output(&self, name: &str) -> Option<&[u8]> {
        self.outputs.get(name).map(Vec::as_slice)
    }

    /// Every written name, in sorted order.
    pub fn output_names(&self) -> impl Iterator<Item = &str> {
        self.outputs.keys().map(String::as_str)
    }

    fn resolve(name: &str, relative_to: Option<&str>) -> String {
        match relative_to {
            Some(base) => match base.rfind('/') {
                Some(cut) => format!("{}/{name}", &base[..cut]),
                None => name.to_owned(),
            },
            None => name.to_owned(),
        }
    }
}

impl SourceLoader for Memory {
    fn load(
        &self,
        name: &str,
        relative_to: Option<SourceId>,
        sources: &mut Sources,
    ) -> Result<SourceId, IoError> {
        let base = relative_to.and_then(|id| sources.get(id).map(|s| s.name().to_owned()));
        let resolved = Self::resolve(name, base.as_deref());
        let bytes = self
            .inputs
            .get(&resolved)
            .ok_or_else(|| IoError::NotFound {
                name: resolved.clone(),
            })?;
        Ok(sources.add(resolved, bytes.clone()))
    }
}

impl OutputSink for Memory {
    fn write(&mut self, name: &str, bytes: &[u8]) -> Result<(), IoError> {
        self.outputs.insert(name.to_owned(), bytes.to_vec());
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_name_resolves_against_the_file_that_wrote_it() {
        let store = Memory::new()
            .with_input("paper/main.tex", b"root".as_slice())
            .with_input("paper/sections/model.tex", b"section".as_slice());
        let mut sources = Sources::new();

        let root = store.load("paper/main.tex", None, &mut sources).unwrap();
        let nested = store
            .load("sections/model.tex", Some(root), &mut sources)
            .unwrap();

        assert_eq!(
            sources.get(nested).unwrap().name(),
            "paper/sections/model.tex"
        );
        assert_eq!(sources.get(nested).unwrap().bytes(), b"section");
    }

    #[test]
    fn a_missing_name_reports_what_was_looked_for() {
        let store = Memory::new();
        let mut sources = Sources::new();

        let err = store.load("nope.tex", None, &mut sources).unwrap_err();

        assert_eq!(
            err,
            IoError::NotFound {
                name: "nope.tex".to_owned()
            }
        );
        assert_eq!(err.to_string(), "not found: nope.tex");
    }
}
