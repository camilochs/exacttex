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
//! [`parse`] recognises the inline constructs and the raw escape, and honours
//! the regions in which an entry token is ordinary text — comments, math,
//! verbatim, and the raw escape itself. Everything else is opaque and
//! transported. Command arguments are still missing from that list, so a
//! construct inside one is recognised when it should not be; that gap closes
//! with the signature database.
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
//! ```

pub mod bibliography;
pub mod blocks;
pub mod check;
pub mod diagnostics;
pub mod document;
pub mod io;
pub mod review;
pub mod scanner;
pub mod signatures;
pub mod source;
pub mod sourcemap;
pub mod symbols;

use std::error::Error;
use std::fmt;

use blocks::{BlockKind, Value, parse_block};
use document::{Document, Node, ParseConfidence};
use io::{IoError, OutputSink, SourceLoader};
use scanner::{EntryToken, Piece};
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

    let mut covered_until = 0usize;
    for piece in scanner::scan(source.bytes()) {
        let piece_span = match piece {
            Piece::Text(span)
            | Piece::Excluded(span)
            | Piece::Construct { span, .. }
            | Piece::Malformed { span, .. } => span,
        };
        if piece_span.start() < covered_until {
            continue;
        }
        covered_until = piece_span.end();
        let node = match piece {
            // Prose the parser modelled nothing in, and a region §8 excludes,
            // are both transported. They differ for a consumer searching the
            // source, not for the emitter.
            Piece::Text(span) | Piece::Excluded(span) => Node::Opaque {
                source: id,
                span,
                // Balanced rather than to-end-of-file: the region has a known
                // end, it is simply not modelled. Nothing has given up.
                confidence: ParseConfidence::OpaqueBalanced,
            },
            Piece::Construct {
                kind,
                span,
                children,
            } => Node::Construct {
                source: id,
                span,
                kind,
                children: children
                    .into_iter()
                    .map(|piece| node_from_piece(piece, id))
                    .collect(),
            },
            Piece::Malformed { kind, span } => Node::Malformed {
                source: id,
                span,
                kind,
            },
        };
        document.push(node);
    }
    document
}

fn node_from_piece(piece: Piece, source: SourceId) -> Node {
    match piece {
        Piece::Text(span) | Piece::Excluded(span) => Node::Opaque {
            source,
            span,
            confidence: ParseConfidence::OpaqueBalanced,
        },
        Piece::Construct {
            kind,
            span,
            children,
        } => Node::Construct {
            source,
            span,
            kind,
            children: children
                .into_iter()
                .map(|piece| node_from_piece(piece, source))
                .collect(),
        },
        Piece::Malformed { kind, span } => Node::Malformed { source, span, kind },
    }
}

/// Writes a document as LaTeX in emission order.
///
/// Opaque and malformed nodes are copied from their source spans. Recognised
/// constructs lower to their specified LaTeX form; braced field content is
/// still copied from its source span rather than decoded or reconstructed.
///
/// # Errors
///
/// Returns [`EmitError`] if a node spans a range its source does not contain,
/// which means the document and the arena disagree.
pub fn emit(sources: &Sources, document: &Document, out: &mut Vec<u8>) -> Result<(), EmitError> {
    emit_view(sources, document, RevisionView::Final, out)
}

/// A document view of the revisions carried in its source.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RevisionView {
    /// The document before every proposed change.
    Original,
    /// The document with every proposed change applied.
    Final,
    /// A review copy in which additions and deletions are visible.
    Marked,
}

/// Writes one revision view as LaTeX.
///
/// # Errors
///
/// Returns [`EmitError`] if a node refers outside its immutable source.
pub fn emit_view(
    sources: &Sources,
    document: &Document,
    view: RevisionView,
    out: &mut Vec<u8>,
) -> Result<(), EmitError> {
    if view == RevisionView::Marked {
        let mut body = Vec::new();
        emit_nodes(sources, document, view, &mut body)?;
        let insertion = documentclass_end(&body).unwrap_or(0);
        out.extend_from_slice(&body[..insertion]);
        out.extend_from_slice(b"\\usepackage{xcolor}\n\\usepackage[normalem]{ulem}\n");
        out.extend_from_slice(&body[insertion..]);
        return Ok(());
    }
    emit_nodes(sources, document, view, out)
}

fn emit_nodes(
    sources: &Sources,
    document: &Document,
    view: RevisionView,
    out: &mut Vec<u8>,
) -> Result<(), EmitError> {
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
        match node {
            Node::Construct { kind, .. } => emit_construct(*kind, bytes, view, out),
            Node::Opaque { .. } | Node::Malformed { .. } => out.extend_from_slice(bytes),
        }
    }
    Ok(())
}

fn documentclass_end(bytes: &[u8]) -> Option<usize> {
    let start = bytes
        .windows(b"\\documentclass".len())
        .position(|window| window == b"\\documentclass")?;
    let mut at = start + b"\\documentclass".len();
    while bytes.get(at).is_some_and(u8::is_ascii_whitespace) {
        at += 1;
    }
    if bytes.get(at) == Some(&b'[') {
        let mut depth = 1u32;
        at += 1;
        while at < bytes.len() && depth > 0 {
            match bytes[at] {
                b'[' => depth += 1,
                b']' => depth -= 1,
                _ => {}
            }
            at += 1;
        }
        while bytes.get(at).is_some_and(u8::is_ascii_whitespace) {
            at += 1;
        }
    }
    let end = scanner::balanced_end(bytes, at)?;
    Some(
        if bytes.get(end) == Some(&b'\r') && bytes.get(end + 1) == Some(&b'\n') {
            end + 2
        } else if bytes.get(end) == Some(&b'\n') {
            end + 1
        } else {
            end
        },
    )
}

fn emit_construct(kind: EntryToken, bytes: &[u8], view: RevisionView, out: &mut Vec<u8>) {
    match kind {
        EntryToken::Id => emit_inline(bytes, b"\\label{", out),
        EntryToken::Ref => emit_inline(bytes, b"\\ref{", out),
        EntryToken::Cite => {
            out.push(b'\\');
            let open = bytes.iter().position(|byte| *byte == b'(').unwrap_or(1);
            out.extend_from_slice(&bytes[1..open]);
            out.push(b'{');
            emit_citation_keys(&bytes[open + 1..bytes.len() - 1], out);
            out.push(b'}');
        }
        EntryToken::Import => emit_import(bytes, view, out),
        EntryToken::Add | EntryToken::Del | EntryToken::Sub | EntryToken::Note => {
            emit_revision(kind, bytes, view, out);
        }
        EntryToken::Raw => {
            let open = bytes.iter().position(|byte| *byte == b'{').unwrap_or(0);
            out.extend_from_slice(&bytes[open + 1..bytes.len() - 1]);
        }
        EntryToken::Figure | EntryToken::Table => emit_block(kind, bytes, view, out),
    }
}

fn emit_revision(kind: EntryToken, bytes: &[u8], view: RevisionView, out: &mut Vec<u8>) {
    let Some(open) = bytes.iter().position(|byte| *byte == b'{') else {
        return;
    };
    let end = bytes.len().saturating_sub(1);
    let (left, right) = if kind == EntryToken::Sub {
        let separator = scanner::substitution_separator(bytes, open + 1, end).unwrap_or(end);
        (
            trim_end(&bytes[open + 1..separator]),
            trim_start(&bytes[(separator + 2).min(end)..end]),
        )
    } else {
        (&bytes[open + 1..end], &[][..])
    };
    match (kind, view) {
        (EntryToken::Add, RevisionView::Final)
        | (EntryToken::Del | EntryToken::Sub, RevisionView::Original) => {
            emit_fragment(left, view, out);
        }
        (EntryToken::Sub, RevisionView::Final) => emit_fragment(right, view, out),
        (EntryToken::Add, RevisionView::Marked) => {
            out.extend_from_slice(b"\\textcolor{blue}{");
            emit_fragment(left, view, out);
            out.push(b'}');
        }
        (EntryToken::Del, RevisionView::Marked) => {
            out.extend_from_slice(b"\\textcolor{red}{\\sout{");
            emit_fragment(left, view, out);
            out.extend_from_slice(b"}}");
        }
        (EntryToken::Sub, RevisionView::Marked) => {
            out.extend_from_slice(b"\\textcolor{red}{\\sout{");
            emit_fragment(left, view, out);
            out.extend_from_slice(b"}}\\textcolor{blue}{");
            emit_fragment(right, view, out);
            out.push(b'}');
        }
        _ => {}
    }
}

fn trim_start(mut bytes: &[u8]) -> &[u8] {
    while bytes.first().is_some_and(u8::is_ascii_whitespace) {
        bytes = &bytes[1..];
    }
    bytes
}

fn trim_end(mut bytes: &[u8]) -> &[u8] {
    while bytes.last().is_some_and(u8::is_ascii_whitespace) {
        bytes = &bytes[..bytes.len() - 1];
    }
    bytes
}

fn emit_fragment(bytes: &[u8], view: RevisionView, out: &mut Vec<u8>) {
    emit_content(bytes, view, out);
}

fn emit_content(bytes: &[u8], view: RevisionView, out: &mut Vec<u8>) {
    let mut covered_until = 0usize;
    for piece in scanner::scan(bytes) {
        let piece_span = match piece {
            Piece::Text(span)
            | Piece::Excluded(span)
            | Piece::Construct { span, .. }
            | Piece::Malformed { span, .. } => span,
        };
        if piece_span.start() < covered_until {
            continue;
        }
        covered_until = piece_span.end();
        let fragment = &bytes[piece_span.start()..piece_span.end()];
        match piece {
            Piece::Construct { kind, .. } => emit_construct(kind, fragment, view, out),
            Piece::Text(_) | Piece::Excluded(_) | Piece::Malformed { .. } => {
                out.extend_from_slice(fragment);
            }
        }
    }
}

fn emit_inline(bytes: &[u8], prefix: &[u8], out: &mut Vec<u8>) {
    let open = bytes.iter().position(|byte| *byte == b'(').unwrap_or(0);
    out.extend_from_slice(prefix);
    out.extend_from_slice(&bytes[open + 1..bytes.len() - 1]);
    out.push(b'}');
}

fn emit_citation_keys(keys: &[u8], out: &mut Vec<u8>) {
    let mut first = true;
    for key in keys.split(|byte| *byte == b',') {
        if !first {
            out.push(b',');
        }
        let start = key
            .iter()
            .position(|byte| !byte.is_ascii_whitespace())
            .unwrap_or(key.len());
        let end = key
            .iter()
            .rposition(|byte| !byte.is_ascii_whitespace())
            .map_or(start, |i| i + 1);
        out.extend_from_slice(&key[start..end]);
        first = false;
    }
}

fn emit_import(bytes: &[u8], view: RevisionView, out: &mut Vec<u8>) {
    let first_quote = bytes.iter().position(|byte| *byte == b'"').unwrap_or(0);
    let last_quote = bytes
        .iter()
        .rposition(|byte| *byte == b'"')
        .unwrap_or(first_quote);
    out.extend_from_slice(b"\\input{");
    let path = &bytes[first_quote + 1..last_quote];
    let stem = path.strip_suffix(b".ntex").unwrap_or(path);
    out.extend_from_slice(stem);
    if view == RevisionView::Marked {
        out.extend_from_slice(b".marked.tex}");
    } else {
        out.extend_from_slice(b".tex}");
    }
}

fn emit_block(token: EntryToken, bytes: &[u8], view: RevisionView, out: &mut Vec<u8>) {
    let (kind, entry_len, environment) = match token {
        EntryToken::Figure => (BlockKind::Figure, b"\\figure(".len(), b"figure".as_slice()),
        EntryToken::Table => (BlockKind::Table, b"\\table(".len(), b"table".as_slice()),
        _ => return,
    };
    let Ok(block) = parse_block(bytes, kind, 0, entry_len) else {
        out.extend_from_slice(bytes);
        return;
    };
    let field = |name: &[u8]| {
        block
            .fields
            .iter()
            .find(|field| bytes.get(field.key.start()..field.key.end()) == Some(name))
    };

    out.extend_from_slice(b"\\begin{");
    out.extend_from_slice(environment);
    out.extend_from_slice(b"}\n");
    if kind == BlockKind::Table || field(b"src").is_some() {
        out.extend_from_slice(b"  \\centering\n");
    }
    if let Some(src) = field(b"src") {
        out.extend_from_slice(b"  \\includegraphics");
        // A percentage becomes a fraction of a reference fixed by the field,
        // never one the author selects. `\\linewidth` is the only width correct
        // both inside a single-column float and inside a spanning one; for
        // height TeX offers no adaptive reference at all. `decisions/0004`.
        let mut options: Vec<u8> = Vec::new();
        for (name, reference) in [
            (&b"width"[..], &b"\\linewidth"[..]),
            (&b"height"[..], &b"\\textheight"[..]),
        ] {
            let Some(field) = field(name) else { continue };
            if !options.is_empty() {
                options.push(b',');
            }
            options.extend_from_slice(name);
            options.push(b'=');
            if matches!(field.value, Value::Percentage(_)) {
                emit_percentage(bytes, field.value.span(), &mut options);
                options.extend_from_slice(reference);
            } else {
                copy_value(bytes, field.value, false, &mut options);
            }
        }
        if !options.is_empty() {
            out.push(b'[');
            out.extend_from_slice(&options);
            out.push(b']');
        }
        out.push(b'{');
        copy_value(bytes, src.value, false, out);
        out.extend_from_slice(b"}\n");
    }
    if let Some(caption) = field(b"caption") {
        out.extend_from_slice(b"  \\caption{");
        emit_braced_content(bytes, caption.value, view, out);
        out.extend_from_slice(b"}\n");
    }
    if let Some(body) = field(b"body") {
        emit_braced_content(bytes, body.value, view, out);
        out.push(b'\n');
    }
    if let Some(trailing) = field(b"trailing") {
        emit_braced_content(bytes, trailing.value, view, out);
        out.push(b'\n');
    }
    out.extend_from_slice(b"  \\label{");
    out.extend_from_slice(&bytes[block.id.start()..block.id.end()]);
    out.extend_from_slice(b"}\n\\end{");
    out.extend_from_slice(environment);
    out.push(b'}');
}

fn emit_braced_content(bytes: &[u8], value: Value, view: RevisionView, out: &mut Vec<u8>) {
    let span = value.span();
    emit_content(&bytes[span.start() + 1..span.end() - 1], view, out);
}

fn emit_percentage(bytes: &[u8], span: source::Span, out: &mut Vec<u8>) {
    let number = &bytes[span.start()..span.end() - 1];
    let dot = number
        .iter()
        .position(|byte| *byte == b'.')
        .unwrap_or(number.len());
    let mut digits = number.to_vec();
    if dot < digits.len() {
        digits.remove(dot);
    }
    while digits.first() == Some(&b'0') && digits.len() > 1 {
        digits.remove(0);
    }
    let fractional = if dot == number.len() {
        0
    } else {
        number.len() - dot - 1
    };
    let decimal = digits.len().saturating_sub(fractional + 2);
    if decimal == 0 {
        out.extend_from_slice(b"0.");
        for _ in digits.len()..fractional + 2 {
            out.push(b'0');
        }
        out.extend_from_slice(&digits);
    } else {
        out.extend_from_slice(&digits[..decimal]);
        if decimal < digits.len() {
            out.push(b'.');
            out.extend_from_slice(&digits[decimal..]);
        }
    }
}

fn copy_value(bytes: &[u8], value: Value, strip_delimiters: bool, out: &mut Vec<u8>) {
    let span = value.span();
    let mut start = span.start();
    let mut end = span.end();
    if strip_delimiters || matches!(value, Value::Str(_)) {
        start += 1;
        end -= 1;
    }
    out.extend_from_slice(&bytes[start..end]);
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

/// Loads `name`, parses it, and emits one LaTeX file.
///
/// For input carrying no NextTeX construct the bytes written are the bytes
/// read. Project traversal and output-name mapping belong to the host front end.
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
    fn latex_carrying_no_construct_is_wholly_opaque() {
        let mut sources = Sources::new();
        let id = sources.add("main.tex", AWKWARD);
        let document = parse(&sources, id);

        // The fixture holds a comment, so the scanner splits it into prose and
        // an excluded region. Both are transported, so both are opaque nodes.
        assert!(!document.is_empty());
        assert!(document.iter().all(Node::is_opaque));
        assert!((document.coverage() - 0.0).abs() < f64::EPSILON);
        assert!(!document.reached_end_of_recognition());
    }

    #[test]
    fn a_construct_raises_coverage_above_zero() {
        let mut sources = Sources::new();
        let id = sources.add("main.ntex", b"See @ref(a).".as_slice());
        let document = parse(&sources, id);

        assert!(document.coverage() > 0.0);
        assert!(document.iter().any(|n| !n.is_opaque()));
    }

    #[test]
    fn braced_field_bytes_are_copied_without_utf8_decoding() {
        let input =
            b"before \\table(t) { caption = {Caf\xE9 \\% {nested}} body = {a\xFF\\\\b} } after";
        let mut sources = Sources::new();
        let id = sources.add("main.ntex", input.as_slice());
        let document = parse(&sources, id);
        let mut out = Vec::new();
        emit(&sources, &document, &mut out).unwrap();

        assert!(out.starts_with(b"before \\begin{table}"));
        for needle in [b"Caf\xE9 \\% {nested}".as_slice(), b"a\xFF\\\\b".as_slice()] {
            assert!(out.windows(needle.len()).any(|part| part == needle));
        }
        assert!(out.ends_with(b"\\end{table} after"));
    }
}
