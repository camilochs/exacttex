//! Immutable source buffers and the spans that index them.
//!
//! A source is held as bytes and is never decoded. Input is not assumed to be
//! valid UTF-8, and nothing here converts it: a span is a pair of byte offsets,
//! and emission copies the indexed slice. This is what the transport property
//! rests on — see `PHILOSOPHY.md` §5.

use std::fmt;
use std::sync::Arc;

/// Handle to a loaded source.
///
/// Opaque and cheap to copy. It carries no path: the core does not know where
/// a source came from, only that something loaded it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SourceId(u32);

impl SourceId {
    /// Index of this source within the arena that produced it.
    #[must_use]
    pub const fn index(self) -> usize {
        self.0 as usize
    }

    #[cfg(test)]
    pub(crate) const fn from_index_for_test(index: u32) -> Self {
        Self(index)
    }
}

impl fmt::Display for SourceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "source#{}", self.0)
    }
}

/// A half-open byte range `[start, end)` within one source.
///
/// Offsets are byte offsets, not character offsets. A span never crosses
/// sources; the [`SourceId`] is carried alongside it by whatever holds the span.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Span {
    start: u32,
    end: u32,
}

impl Span {
    /// Creates a span.
    ///
    /// # Panics
    ///
    /// Panics if `end` is before `start`. A reversed span is a broken internal
    /// invariant rather than a condition callers are expected to handle.
    #[must_use]
    pub const fn new(start: u32, end: u32) -> Self {
        assert!(start <= end, "span end precedes its start");
        Self { start, end }
    }

    /// First byte offset covered by the span.
    #[must_use]
    pub const fn start(self) -> usize {
        self.start as usize
    }

    /// One past the last byte offset covered by the span.
    #[must_use]
    pub const fn end(self) -> usize {
        self.end as usize
    }

    /// Number of bytes the span covers.
    #[must_use]
    pub const fn len(self) -> usize {
        (self.end - self.start) as usize
    }

    /// Whether the span covers no bytes.
    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.start == self.end
    }
}

/// One loaded source: a logical name and the exact bytes behind it.
///
/// The name is whatever the loader used to identify it — a project-relative
/// path, an editor URI, a fixture label. The core neither parses nor resolves
/// it.
#[derive(Debug, Clone)]
pub struct Source {
    id: SourceId,
    name: Arc<str>,
    bytes: Arc<[u8]>,
}

impl Source {
    /// Handle for this source.
    #[must_use]
    pub const fn id(&self) -> SourceId {
        self.id
    }

    /// Logical name the loader gave this source.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The complete byte content.
    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// Bytes covered by `span`, or `None` if the span runs past the end.
    ///
    /// Returning `None` rather than panicking keeps a stale span from aborting
    /// a build; the caller reports it as a broken invariant with a location.
    #[must_use]
    pub fn slice(&self, span: Span) -> Option<&[u8]> {
        self.bytes.get(span.start()..span.end())
    }

    /// Span covering the whole source.
    ///
    /// # Panics
    ///
    /// Panics if the source is larger than `u32::MAX` bytes. Such an input is
    /// rejected at load time; reaching here means the limit was bypassed.
    #[must_use]
    pub fn full_span(&self) -> Span {
        let len = u32::try_from(self.bytes.len()).expect("source exceeds u32 addressing");
        Span::new(0, len)
    }
}

/// Arena of loaded sources.
///
/// Sources are added once and never mutated, so a [`SourceId`] stays valid for
/// the lifetime of the arena and spans into it never dangle.
#[derive(Debug, Default, Clone)]
pub struct Sources {
    entries: Vec<Source>,
}

impl Sources {
    /// Creates an empty arena.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    /// Interns bytes under a logical name and returns its handle.
    ///
    /// # Panics
    ///
    /// Panics if more than `u32::MAX` sources are added.
    pub fn add(&mut self, name: impl Into<Arc<str>>, bytes: impl Into<Arc<[u8]>>) -> SourceId {
        let id = SourceId(u32::try_from(self.entries.len()).expect("too many sources"));
        self.entries.push(Source {
            id,
            name: name.into(),
            bytes: bytes.into(),
        });
        id
    }

    /// The source behind `id`, or `None` if it came from another arena.
    #[must_use]
    pub fn get(&self, id: SourceId) -> Option<&Source> {
        self.entries.get(id.index())
    }

    /// Number of loaded sources.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether no source has been loaded.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Iterates over every loaded source, in load order.
    pub fn iter(&self) -> impl Iterator<Item = &Source> {
        self.entries.iter()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_source_keeps_its_bytes_exactly() {
        // Not valid UTF-8: a lone 0xFF, a Latin-1 e-acute, and a CRLF.
        let raw: &[u8] = b"\\section{Caf\xE9}\r\n\xFF";
        let mut sources = Sources::new();
        let id = sources.add("main.tex", raw);
        let source = sources.get(id).expect("just added");

        assert_eq!(source.bytes(), raw);
        assert_eq!(source.slice(source.full_span()), Some(raw));
    }

    #[test]
    fn a_span_past_the_end_yields_none_rather_than_panicking() {
        let mut sources = Sources::new();
        let id = sources.add("main.tex", b"abc".as_slice());
        let source = sources.get(id).expect("just added");

        assert_eq!(source.slice(Span::new(1, 3)), Some(b"bc".as_slice()));
        assert_eq!(source.slice(Span::new(1, 9)), None);
    }

    #[test]
    fn an_id_from_another_arena_is_not_found() {
        let mut first = Sources::new();
        let mut second = Sources::new();
        second.add("a.tex", b"a".as_slice());
        let stray = second.add("b.tex", b"b".as_slice());

        assert!(first.get(stray).is_none());
        first.add("only.tex", b"x".as_slice());
        assert!(first.get(stray).is_none());
    }
}
