//! Segment source maps for emitted LaTeX.

use crate::document::{Document, Node};
use crate::scanner::EntryToken;
use crate::source::{SourceId, Sources, Span};
use crate::{EmitError, emit};
use std::fmt::Write as _;

/// Which side of emission accounts for an output range.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OriginKind {
    /// LaTeX copied from the author's source.
    AuthorLatex,
    /// Bytes copied from an explicit ExactTeX construct.
    XtexNative,
    /// Bytes synthesized while lowering a ExactTeX construct.
    XtexGenerated,
}

impl OriginKind {
    /// Stable spelling used by diagnostics and map files.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::AuthorLatex => "author-latex",
            Self::XtexNative => "xtex-construct",
            Self::XtexGenerated => "xtex-generated",
        }
    }
}

/// A source range responsible for emitted bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Origin {
    /// Source containing the origin.
    pub source: SourceId,
    /// Range in that source.
    pub span: Span,
    /// Side of the emission boundary.
    pub kind: OriginKind,
}

/// A half-open output range and its origin-table entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MapSegment {
    /// First emitted byte.
    pub output_start: u32,
    /// One past the last emitted byte.
    pub output_end: u32,
    /// Index in [`SourceMap::origins`].
    pub origin: u32,
}

/// One source file recorded in a map.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MapSource {
    /// Logical source name.
    pub name: String,
    /// SHA-256 of its exact bytes.
    pub fingerprint: [u8; 32],
    /// Byte offset of every line start, beginning with zero.
    pub line_starts: Vec<u32>,
}

/// The emitted bytes and the complete segment map produced with them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MappedEmission {
    /// LaTeX bytes. These are identical to [`emit`] output.
    pub bytes: Vec<u8>,
    /// Map for `bytes`.
    pub map: SourceMap,
}

/// Map from output byte ranges to source origins.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceMap {
    /// Fingerprint of the document's primary input.
    pub input_fingerprint: [u8; 32],
    /// Fingerprint of the complete emitted output.
    pub output_fingerprint: [u8; 32],
    /// Loaded source-file table, in `SourceId` order.
    pub sources: Vec<MapSource>,
    /// Origins referenced by segments.
    pub origins: Vec<Origin>,
    /// Ordered, non-overlapping segments covering the output.
    pub segments: Vec<MapSegment>,
}

impl SourceMap {
    /// Finds the segment containing `output_offset` by bisection.
    #[must_use]
    pub fn lookup(&self, output_offset: u32) -> Option<(&MapSegment, &Origin)> {
        let index = self
            .segments
            .partition_point(|segment| segment.output_end <= output_offset);
        let segment = self.segments.get(index)?;
        if output_offset < segment.output_start {
            return None;
        }
        let origin = self.origins.get(segment.origin as usize)?;
        Some((segment, origin))
    }

    /// Converts a one-based output line and column to a byte offset.
    #[must_use]
    pub fn output_offset(&self, output: &[u8], line: u32, column: u32) -> Option<u32> {
        line_column_to_offset(&line_starts(output), output.len(), line, column)
    }

    /// Converts an origin byte offset to a one-based line and byte column.
    #[must_use]
    pub fn source_line_column(&self, source: SourceId, offset: u32) -> Option<(u32, u32)> {
        let entry = self.sources.get(source.index())?;
        offset_to_line_column(&entry.line_starts, offset)
    }

    /// Serializes the companion map as UTF-8 JSON without altering output bytes.
    #[must_use]
    pub fn to_json(&self) -> Vec<u8> {
        let mut text = String::new();
        text.push_str("{\"version\":1,\"input_sha256\":\"");
        push_hex(&mut text, &self.input_fingerprint);
        text.push_str("\",\"output_sha256\":\"");
        push_hex(&mut text, &self.output_fingerprint);
        text.push_str("\",\"sources\":[");
        for (index, source) in self.sources.iter().enumerate() {
            if index != 0 {
                text.push(',');
            }
            text.push_str("{\"name\":\"");
            push_json_string(&mut text, &source.name);
            text.push_str("\",\"sha256\":\"");
            push_hex(&mut text, &source.fingerprint);
            text.push_str("\",\"lines\":[");
            push_numbers(&mut text, &source.line_starts);
            text.push_str("]}");
        }
        text.push_str("],\"origins\":[");
        for (index, origin) in self.origins.iter().enumerate() {
            if index != 0 {
                text.push(',');
            }
            write!(
                text,
                "{{\"source\":{},\"start\":{},\"end\":{},\"kind\":\"{}\"}}",
                origin.source.index(),
                origin.span.start(),
                origin.span.end(),
                origin.kind.as_str()
            )
            .expect("writing to a string cannot fail");
        }
        text.push_str("],\"segments\":[");
        for (index, segment) in self.segments.iter().enumerate() {
            if index != 0 {
                text.push(',');
            }
            write!(
                text,
                "[{}, {}, {}]",
                segment.output_start, segment.output_end, segment.origin
            )
            .expect("writing to a string cannot fail");
        }
        text.push_str("]}\n");
        text.into_bytes()
    }
}

/// Emits LaTeX and builds its segment map in the same pass over document nodes.
///
/// # Errors
///
/// Returns [`EmitError`] when a node refers outside its source buffer.
///
/// # Panics
///
/// Panics when output exceeds the compiler's `u32` addressing limit. The host
/// rejects such inputs before this core API is called.
pub fn emit_with_map(sources: &Sources, document: &Document) -> Result<MappedEmission, EmitError> {
    let mut bytes = Vec::new();
    let mut origins = Vec::new();
    let mut segments = Vec::new();

    for node in document.iter() {
        let start = u32::try_from(bytes.len()).expect("emitted output exceeds u32 addressing");
        let mut one = Document::new(document.source());
        one.push(node.clone());
        emit(sources, &one, &mut bytes)?;
        let end = u32::try_from(bytes.len()).expect("emitted output exceeds u32 addressing");
        if start == end {
            continue;
        }
        let kind = match node {
            Node::Opaque { .. } | Node::Malformed { .. } => OriginKind::AuthorLatex,
            Node::Construct {
                kind: EntryToken::Raw,
                ..
            } => OriginKind::XtexNative,
            Node::Construct { .. } => OriginKind::XtexGenerated,
        };
        let span = if matches!(
            node,
            Node::Construct {
                kind: EntryToken::Raw,
                ..
            }
        ) {
            let source = sources
                .get(node.source())
                .expect("node source was checked by emit");
            let raw = source
                .slice(node.span())
                .expect("node span was checked by emit");
            let open = raw.iter().position(|byte| *byte == b'{').unwrap_or(0);
            Span::new(
                u32::try_from(node.span().start() + open + 1).expect("source exceeds u32"),
                u32::try_from(node.span().end() - 1).expect("source exceeds u32"),
            )
        } else {
            node.span()
        };
        let origin = u32::try_from(origins.len()).expect("too many map origins");
        origins.push(Origin {
            source: node.source(),
            span,
            kind,
        });
        segments.push(MapSegment {
            output_start: start,
            output_end: end,
            origin,
        });
    }

    let primary = sources.get(document.source()).ok_or(EmitError {
        source: document.source(),
        start: 0,
        end: 0,
    })?;
    let source_table = sources
        .iter()
        .map(|source| MapSource {
            name: source.name().to_owned(),
            fingerprint: sha256(source.bytes()),
            line_starts: line_starts(source.bytes()),
        })
        .collect();
    let map = SourceMap {
        input_fingerprint: sha256(primary.bytes()),
        output_fingerprint: sha256(&bytes),
        sources: source_table,
        origins,
        segments,
    };
    Ok(MappedEmission { bytes, map })
}

fn line_starts(bytes: &[u8]) -> Vec<u32> {
    let mut starts = vec![0];
    for (index, byte) in bytes.iter().enumerate() {
        if *byte == b'\n' {
            starts.push(u32::try_from(index + 1).expect("source exceeds u32 addressing"));
        }
    }
    starts
}

fn line_column_to_offset(starts: &[u32], len: usize, line: u32, column: u32) -> Option<u32> {
    let start = *starts.get(line.checked_sub(1)? as usize)?;
    let offset = start.checked_add(column.checked_sub(1)?)?;
    (offset as usize <= len).then_some(offset)
}

fn offset_to_line_column(starts: &[u32], offset: u32) -> Option<(u32, u32)> {
    let line = starts
        .partition_point(|start| *start <= offset)
        .checked_sub(1)?;
    Some((u32::try_from(line).ok()? + 1, offset - starts[line] + 1))
}

fn push_numbers(text: &mut String, numbers: &[u32]) {
    for (index, number) in numbers.iter().enumerate() {
        if index != 0 {
            text.push(',');
        }
        text.push_str(&number.to_string());
    }
}

fn push_hex(text: &mut String, bytes: &[u8]) {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    for byte in bytes {
        text.push(HEX[(byte >> 4) as usize] as char);
        text.push(HEX[(byte & 15) as usize] as char);
    }
}

fn push_json_string(text: &mut String, value: &str) {
    for character in value.chars() {
        match character {
            '"' => text.push_str("\\\""),
            '\\' => text.push_str("\\\\"),
            '\n' => text.push_str("\\n"),
            '\r' => text.push_str("\\r"),
            '\t' => text.push_str("\\t"),
            c if c < ' ' => {
                write!(text, "\\u{:04x}", c as u32).expect("writing to a string cannot fail");
            }
            c => text.push(c),
        }
    }
}

// FIPS 180-4 SHA-256, kept here to preserve the compiler core's zero-dependency boundary.
#[allow(clippy::many_single_char_names, clippy::unreadable_literal)]
fn sha256(input: &[u8]) -> [u8; 32] {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
        0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
        0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
        0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
        0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
        0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
        0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
        0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
        0xc67178f2,
    ];
    let mut data = input.to_vec();
    let bits = (data.len() as u64).wrapping_mul(8);
    data.push(0x80);
    while data.len() % 64 != 56 {
        data.push(0);
    }
    data.extend_from_slice(&bits.to_be_bytes());
    let mut h = [
        0x6a09e667u32,
        0xbb67ae85,
        0x3c6ef372,
        0xa54ff53a,
        0x510e527f,
        0x9b05688c,
        0x1f83d9ab,
        0x5be0cd19,
    ];
    for chunk in data.as_chunks::<64>().0 {
        let mut w = [0u32; 64];
        let (words, _) = chunk.as_chunks::<4>();
        for (i, word) in words.iter().enumerate() {
            w[i] = u32::from_be_bytes(*word);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }
        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh] = h;
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ (!e & g);
            let t1 = hh
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[i])
                .wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(maj);
            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }
        for (slot, value) in h.iter_mut().zip([a, b, c, d, e, f, g, hh]) {
            *slot = slot.wrapping_add(value);
        }
    }
    let mut out = [0u8; 32];
    for (chunk, word) in out.as_chunks_mut::<4>().0.iter_mut().zip(h) {
        *chunk = word.to_be_bytes();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse;

    #[test]
    fn sha256_matches_the_published_empty_vector() {
        let mut actual = String::new();
        push_hex(&mut actual, &sha256(b""));
        assert_eq!(
            actual,
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn missing_offsets_do_not_choose_the_nearest_segment() {
        let map = SourceMap {
            input_fingerprint: [0; 32],
            output_fingerprint: [0; 32],
            sources: Vec::new(),
            origins: vec![Origin {
                source: SourceId::from_index_for_test(0),
                span: Span::new(0, 1),
                kind: OriginKind::AuthorLatex,
            }],
            segments: vec![MapSegment {
                output_start: 2,
                output_end: 3,
                origin: 0,
            }],
        };
        assert!(map.lookup(1).is_none());
        assert!(map.lookup(3).is_none());
    }

    #[test]
    fn mapped_and_plain_emission_are_identical() {
        let mut sources = Sources::new();
        let id = sources.add("paper.xtex", b"a@ref(x)b".as_slice());
        let document = parse(&sources, id);
        let mapped = emit_with_map(&sources, &document).expect("mapped emission");
        let mut plain = Vec::new();
        emit(&sources, &document, &mut plain).expect("plain emission");
        assert_eq!(mapped.bytes, plain);
    }
}
