//! Diagnostics attributed through an emitted-file source map.

use crate::source::Span;
use crate::sourcemap::{OriginKind, SourceMap};

/// Which side of the compiler a diagnostic belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Blame {
    /// LaTeX transported from the source.
    AuthorLatex,
    /// An explicit ExactTeX construct.
    NextTexConstruct,
    /// Output synthesized by the emitter.
    NextTexGenerated,
    /// No map segment supports an attribution.
    Unresolved,
}

impl Blame {
    /// Stable spelling shared by human and JSON renderers.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::AuthorLatex => "author-latex",
            Self::NextTexConstruct => "xtex-construct",
            Self::NextTexGenerated => "xtex-generated",
            Self::Unresolved => "unresolved",
        }
    }
}

/// A location in an author's source file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiagnosticSpan {
    /// Logical source name.
    pub file: String,
    /// Byte offset in `file`.
    pub offset: u32,
    /// Number of source bytes selected.
    pub length: u32,
    /// One-based line.
    pub line: u32,
    /// One-based byte column.
    pub column: u32,
}

/// A TeX message with its emitted location resolved to author source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MappedDiagnostic {
    /// Attribution supported by the map.
    pub blame: Blame,
    /// Source location, absent when mapping is unresolved.
    pub span: Option<DiagnosticSpan>,
    /// TeX's message, unchanged.
    pub message: String,
}

/// Resolves a one-based emitted line and byte column through `map`.
#[must_use]
pub fn map_emitted_diagnostic(
    message: impl Into<String>,
    output: &[u8],
    line: u32,
    column: u32,
    map: &SourceMap,
) -> MappedDiagnostic {
    let message = message.into();
    let Some(offset) = map.output_offset(output, line, column) else {
        return MappedDiagnostic {
            blame: Blame::Unresolved,
            span: None,
            message,
        };
    };
    let Some((segment, origin)) = map.lookup(offset) else {
        return MappedDiagnostic {
            blame: Blame::Unresolved,
            span: None,
            message,
        };
    };
    let source_offset = mapped_offset(segment.output_start, origin.span, origin.kind, offset);
    let Some(source) = map.sources.get(origin.source.index()) else {
        return MappedDiagnostic {
            blame: Blame::Unresolved,
            span: None,
            message,
        };
    };
    let Some((source_line, source_column)) = map.source_line_column(origin.source, source_offset)
    else {
        return MappedDiagnostic {
            blame: Blame::Unresolved,
            span: None,
            message,
        };
    };
    let blame = match origin.kind {
        OriginKind::AuthorLatex => Blame::AuthorLatex,
        OriginKind::NextTexNative => Blame::NextTexConstruct,
        OriginKind::NextTexGenerated => Blame::NextTexGenerated,
    };
    MappedDiagnostic {
        blame,
        span: Some(DiagnosticSpan {
            file: source.name.clone(),
            offset: source_offset,
            length: 1,
            line: source_line,
            column: source_column,
        }),
        message,
    }
}

fn mapped_offset(output_start: u32, source: Span, kind: OriginKind, output_offset: u32) -> u32 {
    let source_start = u32::try_from(source.start()).expect("source offset exceeds u32 addressing");
    if kind == OriginKind::NextTexGenerated {
        return source_start;
    }
    let distance = output_offset - output_start;
    let source_len = u32::try_from(source.len()).expect("source span exceeds u32 addressing");
    source_start + distance.min(source_len.saturating_sub(1))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse;
    use crate::source::Sources;
    use crate::sourcemap::emit_with_map;

    /// The case the whole feature exists for: an error inside bytes ExactTeX
    /// produced must name ExactTeX, not the author.
    ///
    /// Without it the compiler hands the author a TeX error against a line
    /// they never wrote, and the honest answer — "this is mine" — is exactly
    /// what stops it being blamed for every LaTeX error in the document.
    #[test]
    fn an_error_in_generated_bytes_blames_the_emitter() {
        let mut sources = Sources::new();
        let id = sources.add(
            "paper.xtex",
            b"\\figure(fig:x) {\n  src = \"p.pdf\"\n  caption = {C}\n}\n".to_vec(),
        );
        let document = crate::parse(&sources, id);
        let emission = crate::sourcemap::emit_with_map(&sources, &document).expect("emits");

        // The \includegraphics line: bytes no field asked for by name, built
        // by the emitter from the block's fields.
        let line = String::from_utf8_lossy(&emission.bytes)
            .lines()
            .position(|l| l.contains("includegraphics"))
            .expect("the block lowered to an includegraphics")
            + 1;

        let mapped = map_emitted_diagnostic(
            "Undefined control sequence",
            &emission.bytes,
            u32::try_from(line).expect("small"),
            3,
            &emission.map,
        );

        assert_eq!(mapped.blame, Blame::NextTexGenerated);
        assert!(
            mapped.span.is_some(),
            "generated bytes still point back at the construct that produced them"
        );
    }

    #[test]
    fn missing_map_reach_is_explicitly_unresolved() {
        let mut sources = Sources::new();
        let id = sources.add("paper.xtex", b"abc".as_slice());
        let document = parse(&sources, id);
        let mut emission = emit_with_map(&sources, &document).expect("emission");
        emission.map.segments.clear();
        let diagnostic =
            map_emitted_diagnostic("TeX's own message", &emission.bytes, 1, 2, &emission.map);
        assert_eq!(diagnostic.blame, Blame::Unresolved);
        assert_eq!(diagnostic.span, None);
        assert_eq!(diagnostic.message, "TeX's own message");
    }

    #[test]
    fn transported_latex_maps_to_its_author_location() {
        let mut sources = Sources::new();
        let id = sources.add("paper.xtex", b"one\ntwo".as_slice());
        let document = parse(&sources, id);
        let emission = emit_with_map(&sources, &document).expect("emission");
        let diagnostic = map_emitted_diagnostic(
            "Undefined control sequence",
            &emission.bytes,
            2,
            2,
            &emission.map,
        );
        assert_eq!(diagnostic.blame, Blame::AuthorLatex);
        let span = diagnostic.span.expect("mapped span");
        assert_eq!(
            (span.file.as_str(), span.offset, span.line, span.column),
            ("paper.xtex", 5, 2, 2)
        );
    }
}
