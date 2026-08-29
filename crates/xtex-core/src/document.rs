//! The document model: a tree whose leaves index the source rather than hold it.
//!
//! Every node carries a [`Span`] into an immutable buffer, and emission copies
//! the indexed slice. Nothing here decodes, normalises or reconstructs text,
//! which is what makes the transport property hold by construction rather than
//! by discipline — see `PHILOSOPHY.md` §5 and `AGENTS.md` §4.
//!
//! # Why an arena
//!
//! Nodes live in a `Vec` and refer to each other by [`NodeId`]. That is not a
//! micro-optimisation: it keeps the tree free of interior pointers, so the
//! representation can be replaced later — by a green/red tree such as `rowan`,
//! if incremental reparsing turns out to be needed for the language server —
//! without changing how the rest of the compiler walks a document. See
//! `docs/references.md` for the trade-off that decision turns on.

use crate::scanner::EntryToken;
use crate::source::{SourceId, Span};

/// How much of a region the parser was able to bound.
///
/// The parser preserves rather than rejects. When it cannot locate a boundary
/// safely it lowers its confidence instead of guessing, and unfamiliar LaTeX is
/// never an error.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ParseConfidence {
    /// The region's boundaries follow the supported surface grammar.
    Structured,
    /// A known delimiter or terminator bounds the region safely.
    OpaqueBalanced,
    /// No boundary could be recovered. Nothing after this point is recognised.
    OpaqueToEof,
}

impl ParseConfidence {
    /// Whether ExactTeX constructs are still recognised after this region.
    #[must_use]
    pub const fn recognition_continues(self) -> bool {
        !matches!(self, Self::OpaqueToEof)
    }
}

/// Handle to a node within one [`Document`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NodeId(u32);

impl NodeId {
    /// Index of this node within the document that produced it.
    #[must_use]
    pub const fn index(self) -> usize {
        self.0 as usize
    }
}

/// One node of the document tree.
///
/// Only [`Node::Opaque`] exists today, because nothing parses yet and every
/// byte is therefore unmodelled. Structured variants are added as the parser
/// learns to bound them; the emitter's contract does not change when they are,
/// because every variant carries the span it came from.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Node {
    /// Source the compiler does not model. Transported byte for byte.
    ///
    /// This is the transport boundary. Its bytes are never copied into the node
    /// — the span indexes the original buffer, and emission writes that slice.
    /// An emitter that reformats, reindents or re-encodes an opaque region
    /// breaks the transport property silently, on a document that was already
    /// accepted somewhere.
    Opaque {
        /// Source this region came from.
        source: SourceId,
        /// Byte range within that source.
        span: Span,
        /// How much of the region the parser could bound.
        confidence: ParseConfidence,
    },
    /// A recognised ExactTeX construct.
    ///
    /// Its source span contains the values used when lowering it to LaTeX.
    Construct {
        /// Source this construct came from.
        source: SourceId,
        /// Byte range covering the whole construct.
        span: Span,
        /// Which construct it is.
        kind: EntryToken,
        /// Constructs nested in document-content fields.
        children: Vec<Node>,
    },
    /// An entry token whose construct could not be closed.
    ///
    /// The bytes transport unchanged; what differs is that a diagnostic points
    /// here.
    Malformed {
        /// Source this token came from.
        source: SourceId,
        /// Byte range covering the entry token.
        span: Span,
        /// Which construct the token opened.
        kind: EntryToken,
    },
}

impl Node {
    /// Source this node came from.
    #[must_use]
    pub const fn source(&self) -> SourceId {
        match self {
            Self::Opaque { source, .. }
            | Self::Construct { source, .. }
            | Self::Malformed { source, .. } => *source,
        }
    }

    /// Byte range this node covers in its source.
    #[must_use]
    pub const fn span(&self) -> Span {
        match self {
            Self::Opaque { span, .. }
            | Self::Construct { span, .. }
            | Self::Malformed { span, .. } => *span,
        }
    }

    /// Whether this node is transported rather than modelled.
    #[must_use]
    pub const fn is_opaque(&self) -> bool {
        matches!(self, Self::Opaque { .. })
    }
}

/// A parsed document: an ordered sequence of nodes over one source.
///
/// Order is emission order. Walking the roots in sequence and writing each
/// node's bytes reproduces the document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Document {
    source: SourceId,
    nodes: Vec<Node>,
}

impl Document {
    /// Creates an empty document over `source`.
    #[must_use]
    pub const fn new(source: SourceId) -> Self {
        Self {
            source,
            nodes: Vec::new(),
        }
    }

    /// Source this document was parsed from.
    #[must_use]
    pub const fn source(&self) -> SourceId {
        self.source
    }

    /// Appends a node and returns its handle.
    ///
    /// # Panics
    ///
    /// Panics if more than `u32::MAX` nodes are added, which exceeds the
    /// configured resource limit long before it is reachable in practice.
    pub fn push(&mut self, node: Node) -> NodeId {
        let id = NodeId(u32::try_from(self.nodes.len()).expect("too many nodes"));
        self.nodes.push(node);
        id
    }

    /// The node behind `id`, or `None` if it came from another document.
    #[must_use]
    pub fn get(&self, id: NodeId) -> Option<&Node> {
        self.nodes.get(id.index())
    }

    /// Iterates over every node in emission order.
    pub fn iter(&self) -> impl Iterator<Item = &Node> {
        self.nodes.iter()
    }

    /// Visits every node, including constructs nested in block fields.
    pub fn walk(&self, mut visit: impl FnMut(&Node)) {
        fn walk_node(node: &Node, visit: &mut impl FnMut(&Node)) {
            visit(node);
            if let Node::Construct { children, .. } = node {
                for child in children {
                    walk_node(child, visit);
                }
            }
        }
        for node in &self.nodes {
            walk_node(node, &mut visit);
        }
    }

    /// Number of nodes.
    #[must_use]
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    /// Whether the document holds no nodes.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    /// Fraction of the document's bytes that are modelled rather than opaque.
    ///
    /// Returns a value in `0.0..=1.0`. An empty document is fully covered:
    /// there is nothing unchecked in it.
    ///
    /// This is the number `xtex check` reports. Under full annotation the
    /// useful signal is a **drop** — something entered the document that the
    /// compiler does not model — rather than an absolute threshold.
    #[must_use]
    pub fn coverage(&self) -> f64 {
        let total: usize = self.nodes.iter().map(|n| n.span().len()).sum();
        if total == 0 {
            return 1.0;
        }
        let opaque: usize = self
            .nodes
            .iter()
            .filter(|n| n.is_opaque())
            .map(|n| n.span().len())
            .sum();
        #[allow(clippy::cast_precision_loss)]
        {
            1.0 - (opaque as f64 / total as f64)
        }
    }

    /// Whether the parser gave up before the end of the source.
    #[must_use]
    pub fn reached_end_of_recognition(&self) -> bool {
        self.nodes.iter().any(
            |n| matches!(n, Node::Opaque { confidence, .. } if !confidence.recognition_continues()),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::Sources;

    fn one_opaque_document(bytes: &[u8]) -> (Sources, Document) {
        let mut sources = Sources::new();
        let id = sources.add("main.tex", bytes);
        let span = sources.get(id).expect("just added").full_span();
        let mut document = Document::new(id);
        document.push(Node::Opaque {
            source: id,
            span,
            confidence: ParseConfidence::OpaqueBalanced,
        });
        (sources, document)
    }

    #[test]
    fn an_opaque_node_holds_a_span_not_a_copy() {
        let raw: &[u8] = b"\\begin{tikzpicture}\xFF\\end{tikzpicture}";
        let (sources, document) = one_opaque_document(raw);
        let node = document.iter().next().expect("one node");

        // The bytes are reachable only through the source. Nothing in the node
        // owns them, so nothing in the node can have altered them.
        let source = sources.get(node.source()).expect("known source");
        assert_eq!(source.slice(node.span()), Some(raw));
    }

    #[test]
    fn a_fully_opaque_document_reports_no_coverage() {
        let (_, document) = one_opaque_document(b"anything at all");
        assert!((document.coverage() - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn an_empty_document_is_fully_covered() {
        let mut sources = Sources::new();
        let id = sources.add("main.tex", b"".as_slice());
        let document = Document::new(id);
        assert!((document.coverage() - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn recognition_stops_after_an_unbounded_region() {
        let mut sources = Sources::new();
        let id = sources.add("main.tex", b"abc".as_slice());
        let mut document = Document::new(id);
        document.push(Node::Opaque {
            source: id,
            span: Span::new(0, 3),
            confidence: ParseConfidence::OpaqueToEof,
        });

        assert!(document.reached_end_of_recognition());
        assert!(!ParseConfidence::OpaqueToEof.recognition_continues());
        assert!(ParseConfidence::OpaqueBalanced.recognition_continues());
        assert!(ParseConfidence::Structured.recognition_continues());
    }

    #[test]
    fn a_node_id_from_another_document_is_not_found() {
        let (_, first) = one_opaque_document(b"a");
        let (_, second) = one_opaque_document(b"bb");
        let stray = NodeId(9);

        assert!(first.get(stray).is_none());
        assert!(second.get(stray).is_none());
    }
}
