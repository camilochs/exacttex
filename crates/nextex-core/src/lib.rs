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
//! There is no parser yet. [`transport`] is the whole pipeline: it loads a
//! source and emits its bytes. That is a complete implementation of the
//! invariant above for the case where the input has no annotations, and the
//! property test that guards it runs from this commit onward rather than
//! arriving with the rest of the compiler.
//!
//! ```
//! use nextex_core::{io::Memory, transport};
//!
//! let mut store = Memory::new().with_input("main.tex", b"\\section{Hi}\r\n".as_slice());
//! transport("main.tex", &store.clone(), &mut store).unwrap();
//!
//! assert_eq!(store.output("main.tex"), Some(b"\\section{Hi}\r\n".as_slice()));
//! ```

pub mod io;
pub mod source;

use io::{IoError, OutputSink, SourceLoader};
use source::Sources;

/// Loads `name` and emits it unchanged.
///
/// This is the transport half of the compiler with nothing in the middle yet.
/// When a parser and an emitter exist they replace the copy, and this function's
/// contract does not change.
/// For input carrying no NextTeX construct, the bytes written are the bytes read.
///
/// # Errors
///
/// Returns [`IoError`] when the source cannot be loaded or the output cannot be
/// written.
///
/// # Panics
///
/// Panics if the loader returns a handle that is absent from the arena it was
/// given. That is a broken loader contract, not a condition a caller can act
/// on.
pub fn transport(
    name: &str,
    loader: &impl SourceLoader,
    sink: &mut impl OutputSink,
) -> Result<(), IoError> {
    let mut sources = Sources::new();
    let id = loader.load(name, None, &mut sources)?;
    let source = sources
        .get(id)
        .expect("a just-loaded source is present in the arena it was loaded into");
    sink.write(source.name(), source.bytes())
}

#[cfg(test)]
mod tests {
    use super::*;
    use io::Memory;

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
        // shape. Today it exercises the copy; it keeps working once a parser
        // sits in the middle.
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
}
