//! The NextTeX compiler core.
//!
//! NextTeX is a one-directional superset of LaTeX: every valid `.tex` file is
//! valid input, and LaTeX remains the artifact of record. This crate holds the
//! part of the compiler that must run unchanged on a native binary and inside a
//! browser, so it contains no filesystem paths, no current directory, and no
//! process spawning. Everything from the host arrives through [`io`].
//!
//! # The invariant this crate exists to keep
//!
//! For input containing no NextTeX constructs, emission returns the input bytes
//! exactly:
//!
//! ```text
//! emit(parse(u)) == u
//! ```
//!
//! Byte equality, not textual equivalence. Line endings, encodings, comments and
//! whitespace are preserved, because the tree indexes an immutable buffer and
//! emission copies the indexed slice rather than reconstructing text. See
//! `PHILOSOPHY.md` §5 and `AGENTS.md` §4.
//!
//! # Status
//!
//! [`parse`] recognises nothing yet: it produces one opaque node covering the
//! whole source. That is the correct starting point rather than a placeholder,
//! because the parser's job is to *narrow* what is opaque, and every construct
//! it learns to bound shrinks that region without changing the emitter's
//! contract.
//!
//! ```
//! use nextex_core::{emit, parse, source::Sources};
//!
//! let mut sources = Sources::new();
//! let id = sources.add("main.tex", b"\\section{Hi}\r\n".as_slice());
//!
//! let document = parse(&sources, id);
//! let mut out = Vec::new();
//! emit(&sources, &document, &mut out).unwrap();
//!
//! assert_eq!(out, b"\\section{Hi}\r\n");
//! assert_eq!(document.coverage(), 0.0); // nothing is modelled yet
//! ```

pub mod document;
pub mod io;
pub mod source;

use std::error::Error;
use std::fmt;

use document::{Document, Node, ParseConfidence};
use io::{IoError, OutputSink, SourceLoader};
use source::{SourceId, Sources};

/// A node referred to a source range that is not there.
///
/// This is a broken internal invariant rather than a condition callers can act
/// on, so it is reported with the location that makes it reproducible instead
/// of aborting the build.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmitError {
    /// Source the node claimed to come from.
    pub source: SourceId,
    /// First byte offset the node claimed to cover.
    pub start: usize,
    /// One past the last byte offset the node claimed to cover.
    pub end: usize,
}

impl fmt::Display for EmitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "node spans {}..{} of {}, which is not present",
            self.start, self.end, self.source
        )
    }
}

impl Error for EmitError {}

/// Builds a document from a source.
///
/// Nothing is recognised yet, so the result is one opaque node covering the
/// whole source. As the parser learns to bound constructs that region narrows;
/// what stays opaque is transported unchanged, which is why unfamiliar LaTeX is
/// never an error.
#[must_use]
pub fn parse(sources: &Sources, id: SourceId) -> Document {
    let mut document = Document::new(id);
    let Some(source) = sources.get(id) else {
        return document;
    };
    let span = source.full_span();
    if !span.is_empty() {
        document.push(Node::Opaque {
            source: id,
            span,
            // Balanced rather than to-end-of-file: the region has a known end,
            // it is simply not modelled. Nothing has given up.
            confidence: ParseConfidence::OpaqueBalanced,
        });
    }
    document
}

/// Writes a document's bytes in emission order.
///
/// Every node's bytes are copied from the source slice its span indexes. No
/// node's content is reconstructed, so an emitter cannot reformat what it was
/// asked to transport.
///
/// # Errors
///
/// Returns [`EmitError`] if a node spans a range its source does not contain,
/// which means the document and the arena disagree.
pub fn emit(sources: &Sources, document: &Document, out: &mut Vec<u8>) -> Result<(), EmitError> {
    for node in document.iter() {
        let span = node.span();
        let bytes = sources
            .get(node.source())
            .and_then(|source| source.slice(span))
            .ok_or(EmitError {
                source: node.source(),
                start: span.start(),
                end: span.end(),
            })?;
        out.extend_from_slice(bytes);
    }
    Ok(())
}

/// What went wrong running the pipeline over one source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BuildError {
    /// The host could not supply or accept bytes.
    Io(IoError),
    /// The document and the source arena disagreed.
    Emit(EmitError),
}

impl fmt::Display for BuildError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(e) => write!(f, "{e}"),
            Self::Emit(e) => write!(f, "{e}"),
        }
    }
}

impl Error for BuildError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io(e) => Some(e),
            Self::Emit(e) => Some(e),
        }
    }
}

impl From<IoError> for BuildError {
    fn from(error: IoError) -> Self {
        Self::Io(error)
    }
}

impl From<EmitError> for BuildError {
    fn from(error: EmitError) -> Self {
        Self::Emit(error)
    }
}

/// Loads `name`, parses it, and emits the result.
///
/// The pipeline is load, parse, emit. For input carrying no NextTeX construct
/// the bytes written are the bytes read, and that holds while the middle of the
/// pipeline grows.
///
/// # Errors
///
/// Returns [`BuildError`] when the host refuses the read or the write, or when
/// the parsed document disagrees with the arena it was parsed from.
pub fn transport(
    name: &str,
    loader: &impl SourceLoader,
    sink: &mut impl OutputSink,
) -> Result<(), BuildError> {
    let mut sources = Sources::new();
    let id = loader.load(name, None, &mut sources)?;
    let document = parse(&sources, id);

    let mut out = Vec::new();
    emit(&sources, &document, &mut out)?;

    let emitted_name = sources
        .get(id)
        .map_or_else(|| name.to_owned(), |source| source.name().to_owned());
    sink.write(&emitted_name, &out)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use io::Memory;
    use source::Span;

    /// Bytes chosen to break anything that decodes, normalises or re-encodes:
    /// a lone `0xFF`, Latin-1 text, a CRLF, a trailing tab and no final newline.
    const AWKWARD: &[u8] = b"\\section{Caf\xE9}\r\n%% comment\t\n\\ref{a}\xFF  ";

    #[test]
    fn untouched_latex_comes_out_byte_identical() {
        let mut store = Memory::new().with_input("main.tex", AWKWARD);
        transport("main.tex", &store.clone(), &mut store).unwrap();

        assert_eq!(store.output("main.tex"), Some(AWKWARD));
    }

    #[test]
    fn transport_holds_for_every_truncation_of_an_awkward_input() {
        // Truncating at each byte boundary is the cheapest generator that finds
        // real boundary bugs: it manufactures unterminated constructs of every
        // shape. It runs through the tree, so it keeps working as the parser
        // learns to bound things.
        for cut in 0..=AWKWARD.len() {
            let slice = &AWKWARD[..cut];
            let mut store = Memory::new().with_input("main.tex", slice);
            transport("main.tex", &store.clone(), &mut store).unwrap();

            assert_eq!(
                store.output("main.tex"),
                Some(slice),
                "transport changed bytes when truncated at {cut}"
            );
        }
    }

    #[test]
    fn transport_holds_for_every_single_byte_input() {
        for byte in 0u8..=255 {
            let input = [byte];
            let mut store = Memory::new().with_input("main.tex", input.as_slice());
            transport("main.tex", &store.clone(), &mut store).unwrap();

            assert_eq!(
                store.output("main.tex"),
                Some(input.as_slice()),
                "transport changed the single byte {byte:#04x}"
            );
        }
    }

    #[test]
    fn an_empty_source_transports_to_an_empty_output() {
        let mut store = Memory::new().with_input("main.tex", b"".as_slice());
        transport("main.tex", &store.clone(), &mut store).unwrap();

        assert_eq!(store.output("main.tex"), Some(b"".as_slice()));
    }

    #[test]
    fn the_core_reaches_the_host_only_through_the_traits() {
        // The whole pipeline runs against an in-memory store: no filesystem, no
        // current directory, no process. This is the property that lets the same
        // crate compile to WebAssembly, and it is asserted rather than assumed.
        let mut store = Memory::new().with_input("a/b/main.tex", AWKWARD);
        transport("a/b/main.tex", &store.clone(), &mut store).unwrap();

        assert_eq!(store.output("a/b/main.tex"), Some(AWKWARD));
        assert_eq!(
            store.output_names().collect::<Vec<_>>(),
            vec!["a/b/main.tex"]
        );
    }

    #[test]
    fn emission_reassembles_a_split_region_exactly() {
        // Two nodes covering halves of one source must reassemble byte for byte.
        // This is what distinguishes a tree walk from a whole-buffer copy, and
        // it is the property the parser relies on as it splits the opaque region
        // into smaller ones.
        let mut sources = Sources::new();
        let id = sources.add("main.tex", AWKWARD);
        let cut = u32::try_from(AWKWARD.len() / 2).unwrap();
        let total = u32::try_from(AWKWARD.len()).unwrap();

        let mut document = Document::new(id);
        for span in [Span::new(0, cut), Span::new(cut, total)] {
            document.push(Node::Opaque {
                source: id,
                span,
                confidence: ParseConfidence::OpaqueBalanced,
            });
        }

        let mut out = Vec::new();
        emit(&sources, &document, &mut out).unwrap();
        assert_eq!(out, AWKWARD);
    }

    #[test]
    fn emission_reassembles_a_region_split_at_every_boundary() {
        let mut sources = Sources::new();
        let id = sources.add("main.tex", AWKWARD);
        let total = u32::try_from(AWKWARD.len()).unwrap();

        for cut in 0..=total {
            let mut document = Document::new(id);
            document.push(Node::Opaque {
                source: id,
                span: Span::new(0, cut),
                confidence: ParseConfidence::OpaqueBalanced,
            });
            document.push(Node::Opaque {
                source: id,
                span: Span::new(cut, total),
                confidence: ParseConfidence::OpaqueBalanced,
            });

            let mut out = Vec::new();
            emit(&sources, &document, &mut out).unwrap();
            assert_eq!(out, AWKWARD, "split at {cut} did not reassemble");
        }
    }

    #[test]
    fn a_node_spanning_past_its_source_is_reported_not_ignored() {
        let mut sources = Sources::new();
        let id = sources.add("main.tex", b"abc".as_slice());
        let mut document = Document::new(id);
        document.push(Node::Opaque {
            source: id,
            span: Span::new(0, 99),
            confidence: ParseConfidence::OpaqueBalanced,
        });

        let mut out = Vec::new();
        let error = emit(&sources, &document, &mut out).unwrap_err();

        assert_eq!((error.start, error.end), (0, 99));
        assert!(out.is_empty(), "nothing is written when a span is broken");
    }

    #[test]
    fn nothing_is_modelled_yet_so_coverage_is_zero() {
        let mut sources = Sources::new();
        let id = sources.add("main.tex", AWKWARD);
        let document = parse(&sources, id);

        assert_eq!(document.len(), 1);
        assert!(document.iter().all(Node::is_opaque));
        assert!((document.coverage() - 0.0).abs() < f64::EPSILON);
        assert!(!document.reached_end_of_recognition());
    }
}
