//! Where citation keys come from, and when we may say one is missing.
//!
//! `@cite(k)` can only be checked against a key set, and a LaTeX document
//! declares its bibliography in three ways. Two point at a `.bib` file; the
//! third puts the entries in the document itself.
//!
//! # The rule that keeps this honest
//!
//! Any failure makes the whole document's state [`Bibliography::Unavailable`]
//! rather than yielding a partial key set. A partial set produces false
//! "missing key" errors, which is worse than no checking: it teaches an author
//! to distrust the diagnostic, and then the real missing key is ignored too.
//!
//! Under `Unavailable`, a citation gets an advisory saying checking was not
//! possible. It never gets a missing-key error, and the exit code does not
//! change.
//!
//! # What this does not check
//!
//! That a key **exists**. Not that the entry behind it is real. An entry with a
//! correct title, a DOI that resolves and a fabricated author list satisfies
//! every check here, because from inside the document that key exists and is
//! spelled correctly. Verifying an entry against Crossref or arXiv is a
//! different job and needs the network.

use std::collections::{BTreeMap, BTreeSet};

use crate::source::{SourceId, Sources, Span};
use crate::symbols::{Reference, SymbolTable};

/// Why a key set could not be assembled.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Unavailable {
    /// The document declares no bibliography at all.
    NoneDeclared,
    /// A declared path is computed rather than literal, so it cannot be read.
    ComputedPath {
        /// Where the declaration is.
        span: Span,
    },
    /// A declared resource could not be read.
    Unreadable {
        /// The resource that was requested.
        name: String,
    },
    /// An entry's boundary could not be located.
    UnparsableEntry {
        /// The resource the entry is in.
        name: String,
        /// A location and description that can be printed without a source map.
        detail: String,
    },
}

impl Unavailable {
    /// Why the key set is missing, in the words a reader of the diagnostic needs.
    ///
    /// The derived `Debug` spelling carries field names and byte offsets, which
    /// are the compiler's business rather than the author's.
    #[must_use]
    pub fn reason(&self) -> String {
        match self {
            Self::NoneDeclared => "the document declares no bibliography".to_owned(),
            Self::ComputedPath { .. } => {
                "the declared bibliography path is computed rather than literal".to_owned()
            }
            Self::Unreadable { name } => format!("`{name}` could not be read"),
            Self::UnparsableEntry { name, detail } => {
                format!("`{name}` did not parse — {detail}")
            }
        }
    }
}

/// What the document's bibliography amounts to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Bibliography {
    /// Every declared resource was read and every entry parsed.
    Complete(BTreeSet<String>),
    /// Something failed, so no key may be called missing.
    Unavailable(Unavailable),
}

impl Bibliography {
    /// Whether `key` is absent from a key set that is known to be complete.
    ///
    /// Returns `false` under [`Bibliography::Unavailable`], because absence of
    /// evidence is not evidence of absence and the caller must not report a
    /// missing key it cannot substantiate.
    #[must_use]
    pub fn is_missing(&self, key: &str) -> bool {
        match self {
            Self::Complete(keys) => !keys.contains(key),
            Self::Unavailable(_) => false,
        }
    }

    /// The keys, when they are known.
    #[must_use]
    pub const fn keys(&self) -> Option<&BTreeSet<String>> {
        match self {
            Self::Complete(keys) => Some(keys),
            Self::Unavailable(_) => None,
        }
    }
}

/// A `.bib` file the document says it uses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resource {
    /// The path as the document wrote it, with `.bib` appended if it lacked it.
    pub name: String,
    /// The declaration it came from, for a diagnostic.
    pub span: Span,
}

/// What a document declares about its bibliography.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Declared {
    /// External `.bib` files, in declaration order.
    pub resources: Vec<Resource>,
    /// Keys given by `\bibitem` inside the document itself.
    pub inline_keys: BTreeSet<String>,
    /// A declaration whose path could not be read literally.
    pub computed: Option<Span>,
}

/// Reads what `source` declares about its bibliography.
///
/// Three forms are recognised, because real documents use all three. Measured
/// across 224 files: 39 point at a `.bib`, and 14 hold their entries inline
/// with 501 `\bibitem` between them.
///
/// A declaration counts only from a recognition region. One inside a comment, a
/// verbatim block or a macro body declares nothing, which is the same rule that
/// governs every other construct.
#[must_use]
pub fn declared_in(sources: &Sources, id: SourceId) -> Declared {
    let mut found = Declared::default();
    let Some(source) = sources.get(id) else {
        return found;
    };
    let bytes = source.bytes();

    // Prose, plus the two commands this reader is entitled to look inside.
    // `\bibitem` is included because a `thebibliography` environment holds them
    // and they are declarations there.
    for span in crate::scanner::readable_for(
        bytes,
        &["bibliography", "addbibresource", "bibitem", "begin"],
    ) {
        let region = &bytes[span.start()..span.end()];
        let base = span.start();

        collect_resources(region, base, b"\\bibliography{", &mut found, true);
        collect_resources(region, base, b"\\addbibresource{", &mut found, false);
        collect_bibitems(region, &mut found);
    }
    found
}

/// Finds `\bibliography{a,b}` or `\addbibresource{a.bib}` in one region.
fn collect_resources(
    region: &[u8],
    base: usize,
    keyword: &[u8],
    found: &mut Declared,
    comma_separated: bool,
) {
    let mut at = 0usize;
    while let Some(hit) = find(region, at, keyword) {
        let open = hit + keyword.len();
        let Some(close) = region[open..]
            .iter()
            .position(|b| *b == b'}')
            .map(|i| open + i)
        else {
            at = open;
            continue;
        };
        let argument = &region[open..close];
        let span = Span::new(
            u32::try_from(base + hit).unwrap_or(u32::MAX),
            u32::try_from(base + close + 1).unwrap_or(u32::MAX),
        );

        // A path built from a macro cannot be resolved without expanding it,
        // and expanding is what this compiler does not do.
        if argument.contains(&b'\\') || argument.contains(&b'{') || argument.is_empty() {
            found.computed = Some(span);
            at = close + 1;
            continue;
        }

        let items: Vec<&[u8]> = if comma_separated {
            argument.split(|b| *b == b',').collect()
        } else {
            vec![argument]
        };
        for item in items {
            let trimmed = trim(item);
            if trimmed.is_empty() {
                found.computed = Some(span);
                continue;
            }
            let Ok(text) = std::str::from_utf8(trimmed) else {
                found.computed = Some(span);
                continue;
            };
            // `\bibliography{refs}` names the file without its extension;
            // `\addbibresource{refs.bib}` with it. Both must reach the same
            // name, and a `.BIB` written on a case-insensitive file system is
            // the same file as a `.bib`.
            let has_extension = std::path::Path::new(text)
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("bib"));
            let name = if has_extension {
                text.to_owned()
            } else {
                format!("{text}.bib")
            };
            found.resources.push(Resource { name, span });
        }
        at = close + 1;
    }
}

/// Finds `\bibitem{key}` and `\bibitem[label]{key}` in one region.
///
/// Both forms occur: 483 without a label and 18 with, across the measured
/// corpus. A reader that handled only the common one would silently lose the
/// keys of the other.
fn collect_bibitems(region: &[u8], found: &mut Declared) {
    let keyword = b"\\bibitem";
    let mut at = 0usize;
    while let Some(hit) = find(region, at, keyword) {
        let mut cursor = hit + keyword.len();
        // An optional [label] may precede the key.
        if region.get(cursor) == Some(&b'[') {
            let Some(i) = region[cursor..].iter().position(|b| *b == b']') else {
                at = cursor;
                continue;
            };
            cursor += i + 1;
        }
        if region.get(cursor) != Some(&b'{') {
            at = cursor;
            continue;
        }
        let open = cursor + 1;
        let Some(close) = region[open..]
            .iter()
            .position(|b| *b == b'}')
            .map(|i| open + i)
        else {
            at = open;
            continue;
        };
        if let Ok(key) = std::str::from_utf8(trim(&region[open..close]))
            && !key.is_empty()
        {
            found.inline_keys.insert(key.to_owned());
        }
        at = close + 1;
    }
}

/// Assembles the key set from what was declared and what could be read.
///
/// `read` is asked for each external resource by name. It returns the file's
/// bytes, or `None` when the host cannot supply them.
#[must_use]
pub fn assemble(
    declared: &Declared,
    mut read: impl FnMut(&str) -> Option<Vec<u8>>,
) -> Bibliography {
    if let Some(span) = declared.computed {
        return Bibliography::Unavailable(Unavailable::ComputedPath { span });
    }
    if declared.resources.is_empty() && declared.inline_keys.is_empty() {
        return Bibliography::Unavailable(Unavailable::NoneDeclared);
    }

    let mut keys = declared.inline_keys.clone();
    for resource in &declared.resources {
        let Some(bytes) = read(&resource.name) else {
            return Bibliography::Unavailable(Unavailable::Unreadable {
                name: resource.name.clone(),
            });
        };
        match scan_bib(&bytes) {
            Ok(found) => keys.extend(found),
            Err(detail) => {
                return Bibliography::Unavailable(Unavailable::UnparsableEntry {
                    name: resource.name.clone(),
                    detail,
                });
            }
        }
    }
    Bibliography::Complete(keys)
}

/// Keys declared by a `.bib` file.
///
/// An entry begins `@type{key,` or `@type(key,`. `@comment`, `@preamble` and
/// `@string` declare no citation key and are skipped.
#[must_use]
pub fn keys_in_bib(bytes: &[u8]) -> Option<BTreeSet<String>> {
    scan_bib(bytes).ok()
}

fn scan_bib(bytes: &[u8]) -> Result<BTreeSet<String>, String> {
    Ok(scan_bib_entries(bytes)?
        .into_iter()
        .map(|(key, _)| key)
        .collect())
}

/// Citation commands a reader may look inside: `\cite` and the natbib and
/// biblatex families. `cite` covers every `\cite…` variant by prefix; the
/// rest carry the word later in their name, where a prefix cannot reach it.
const CITATION_COMMANDS: &[&str] = &[
    "cite",
    "Cite",
    "parencite",
    "Parencite",
    "textcite",
    "Textcite",
    "autocite",
    "Autocite",
    "footcite",
    "smartcite",
    "nocite",
];

/// The plain-LaTeX citation key under `offset`: a `\cite{a,b}` (or any of
/// [`CITATION_COMMANDS`], starred, with optional `[…]` arguments) whose
/// braces cover the offset, answered as the one key the cursor is on.
///
/// Plain `\cite` is the author's LaTeX, outside any ExactTeX construct, and
/// the checker never reports its keys. Navigation asks a different question
/// — "where is this defined?" — and the answer is the same either way. The
/// offset must fall in one of [`crate::scanner::readable_for`]'s regions,
/// same prose and same skips as every other reader, so a commented-out
/// citation stays silent here exactly as it does everywhere else. (The
/// command's shape is read from the bytes directly, because the scanner
/// splits a starred or bracketed `\citep*[…]{…}` across two regions.)
#[must_use]
pub fn latex_citation_key_at(bytes: &[u8], offset: usize) -> Option<String> {
    crate::scanner::readable_for(bytes, CITATION_COMMANDS)
        .into_iter()
        .find(|region| offset >= region.start() && offset < region.end())?;

    // The brace group covering the offset. Keys carry no braces of their
    // own, so the nearest brace to the left decides: a `}` means the offset
    // sits between groups, not inside one.
    let open = bytes[..offset]
        .iter()
        .rposition(|&b| b == b'{' || b == b'}')?;
    if bytes[open] != b'{' {
        return None;
    }
    let close = open + bytes[open..].iter().position(|&b| b == b'}')?;
    if offset >= close {
        return None;
    }

    // Walk left from the brace over what a citation may carry — up to two
    // `[…]` arguments and a star — to the command that owns the group.
    let mut cursor = open;
    for _ in 0..2 {
        if cursor > 0 && bytes[cursor - 1] == b']' {
            cursor = bytes[..cursor - 1].iter().rposition(|&b| b == b'[')?;
        }
    }
    if cursor > 0 && bytes[cursor - 1] == b'*' {
        cursor -= 1;
    }
    let name_end = cursor;
    while cursor > 0 && bytes[cursor - 1].is_ascii_alphabetic() {
        cursor -= 1;
    }
    if cursor == name_end || cursor == 0 || bytes[cursor - 1] != b'\\' {
        return None;
    }
    let name = &bytes[cursor..name_end];
    if !CITATION_COMMANDS
        .iter()
        .any(|command| name.starts_with(command.as_bytes()))
    {
        return None;
    }

    // The one key the cursor is on, out of a comma-separated list. A cursor
    // sitting exactly on a comma is on no key, and no key is the answer.
    let mut start = open + 1;
    let mut boundary = start;
    while boundary <= close {
        if boundary < close && bytes[boundary] != b',' {
            boundary += 1;
            continue;
        }
        if offset >= start && offset < boundary {
            let mut from = start;
            let mut to = boundary;
            while from < to && bytes[from].is_ascii_whitespace() {
                from += 1;
            }
            while to > from && bytes[to - 1].is_ascii_whitespace() {
                to -= 1;
            }
            if from == to {
                return None;
            }
            return Some(String::from_utf8_lossy(&bytes[from..to]).into_owned());
        }
        start = boundary + 1;
        boundary += 1;
    }
    None
}

/// The span of `key`'s own token in a `.bib` file, when the file declares it.
///
/// This is a citation's definition site: the one place a `@cite` can send
/// the editor. The scan is [`scan_bib`]'s — same entries, same skips — so a
/// key this finds is exactly a key the checker accepts.
#[must_use]
pub fn entry_span_in_bib(bytes: &[u8], key: &str) -> Option<Span> {
    scan_bib_entries(bytes)
        .ok()?
        .into_iter()
        .find(|(found, _)| found == key)
        .map(|(_, span)| span)
}

fn scan_bib_entries(bytes: &[u8]) -> Result<Vec<(String, Span)>, String> {
    let mut keys = Vec::new();
    let mut at = 0usize;
    while at < bytes.len() {
        if bytes[at] != b'@' {
            at += 1;
            continue;
        }
        let type_start = at + 1;
        let mut cursor = type_start;
        while matches!(bytes.get(cursor), Some(b) if b.is_ascii_alphabetic()) {
            cursor += 1;
        }
        let entry_type = bytes[type_start..cursor].to_ascii_lowercase();
        cursor = skip_space(bytes, cursor);
        let opener = match bytes.get(cursor) {
            Some(b'{') => (b'{', b'}'),
            Some(b'(') => (b'(', b')'),
            _ => {
                at = type_start;
                continue;
            }
        };
        let entry_line = line_at(bytes, cursor);
        cursor += 1;
        // `@comment` is the one entry type BibTeX does not read. It skips to the
        // next `@` and resumes there, so an unbalanced brace inside a comment is
        // not an error, and an entry a writer commented out by wrapping it is
        // still a database entry. Measured against BibTeX 0.99e, both ways:
        // `@comment{a stray { brace}` is accepted, and a `@book` inside a
        // `@comment` still resolves when cited.
        if entry_type == b"comment" {
            at = cursor;
            continue;
        }
        let body_start = cursor;
        let close = entry_end(
            bytes,
            cursor,
            opener.0,
            opener.1,
            entry_line,
            entry_type == b"preamble",
        )?;
        let key_start = skip_space(bytes, cursor);
        let mut key_end = key_start;
        while matches!(bytes.get(key_end), Some(b) if !b.is_ascii_whitespace() && *b != b',' && *b != opener.1)
        {
            key_end += 1;
        }
        if !matches!(entry_type.as_slice(), b"preamble" | b"string") {
            validate_field_separators(bytes, body_start, close)?;
        }
        if !matches!(entry_type.as_slice(), b"preamble" | b"string")
            && key_end > key_start
            && let Ok(key) = std::str::from_utf8(&bytes[key_start..key_end])
        {
            #[allow(clippy::cast_possible_truncation)] // a .bib past 4GB is not a .bib
            keys.push((key.to_owned(), Span::new(key_start as u32, key_end as u32)));
        }
        at = close + 1;
    }
    Ok(keys)
}

fn entry_end(
    bytes: &[u8],
    mut at: usize,
    open: u8,
    close: u8,
    entry_line: usize,
    mut value_position: bool,
) -> Result<usize, String> {
    let mut braces = Vec::new();
    let mut quote = None;
    while let Some(&byte) = bytes.get(at) {
        if quote.is_some() && byte == b'"' && braces.is_empty() {
            quote = None;
        } else if quote.is_none() && value_position && byte == b'"' && braces.is_empty() {
            quote = Some(line_at(bytes, at));
            value_position = false;
        } else if byte == b'{' {
            braces.push(line_at(bytes, at));
        } else if byte == b'}' && !braces.is_empty() {
            braces.pop();
        } else if byte == close && braces.is_empty() && quote.is_none() {
            return Ok(at);
        } else if braces.is_empty() && quote.is_none() {
            if matches!(byte, b'=' | b'#') {
                value_position = true;
            } else if !byte.is_ascii_whitespace() {
                value_position = false;
            }
        }
        at += 1;
    }
    if let Some(line) = quote {
        Err(format!(
            "a quoted value opened at line {line} is never closed"
        ))
    } else if let Some(line) = braces.first() {
        Err(format!("a value opened at line {line} is never closed"))
    } else {
        let delimiter = char::from(open);
        Err(format!(
            "an entry delimiter `{delimiter}` opened at line {entry_line} is never closed"
        ))
    }
}

fn validate_field_separators(bytes: &[u8], start: usize, end: usize) -> Result<(), String> {
    let mut at = start;
    let mut braces = 0usize;
    let mut quoted = false;
    while at < end {
        match bytes[at] {
            b'"' if braces == 0 => quoted = !quoted,
            b'{' if !quoted => braces += 1,
            b'}' if !quoted => braces = braces.saturating_sub(1),
            _ => {}
        }
        if braces == 0
            && !quoted
            && matches!(bytes[at], b'}' | b'"' | b'0'..=b'9' | b'a'..=b'z' | b'A'..=b'Z')
        {
            let mut next = at + 1;
            while next < end && bytes[next].is_ascii_whitespace() {
                next += 1;
            }
            let value_just_closed = matches!(bytes[at], b'}' | b'"');
            if (value_just_closed || next > at + 1)
                && bytes.get(next).is_some_and(u8::is_ascii_alphabetic)
            {
                let mut equals = next + 1;
                while equals < end
                    && (bytes[equals].is_ascii_alphanumeric() || bytes[equals] == b'_')
                {
                    equals += 1;
                }
                equals = skip_space(bytes, equals);
                if bytes.get(equals) == Some(&b'=') {
                    return Err(format!(
                        "two fields at line {} have no comma between them",
                        line_at(bytes, next)
                    ));
                }
            }
        }
        at += 1;
    }
    Ok(())
}

fn line_at(bytes: &[u8], at: usize) -> usize {
    let mut line = 1;
    for &byte in &bytes[..at.min(bytes.len())] {
        if byte == b'\n' {
            line += 1;
        }
    }
    line
}

fn find(haystack: &[u8], from: usize, needle: &[u8]) -> Option<usize> {
    if from >= haystack.len() {
        return None;
    }
    haystack[from..]
        .windows(needle.len())
        .position(|w| w == needle)
        .map(|i| from + i)
}

fn trim(bytes: &[u8]) -> &[u8] {
    let start = bytes.iter().position(|b| !b.is_ascii_whitespace());
    match start {
        None => &[],
        Some(start) => {
            let end = bytes
                .iter()
                .rposition(|b| !b.is_ascii_whitespace())
                .unwrap_or(start);
            &bytes[start..=end]
        }
    }
}

fn skip_space(bytes: &[u8], mut at: usize) -> usize {
    while matches!(bytes.get(at), Some(b) if b.is_ascii_whitespace()) {
        at += 1;
    }
    at
}

/// Convenience for a host that has the files in memory.
#[must_use]
pub fn assemble_from(declared: &Declared, files: &BTreeMap<String, Vec<u8>>) -> Bibliography {
    assemble(declared, |name| files.get(name).cloned())
}

/// Every `@cite` whose key the bibliography does not contain.
///
/// Yields nothing under [`Bibliography::Unavailable`]. That is the whole point
/// of the distinction: a bibliography that could not be read says nothing about
/// which keys exist, and reporting its citations as absent would blame the
/// author for the reader's failure.
pub fn missing_citations<'a>(
    table: &'a SymbolTable,
    bibliography: &'a Bibliography,
) -> impl Iterator<Item = (&'a str, &'a Reference)> {
    table
        .citations()
        .filter(move |(key, _)| bibliography.is_missing(key))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn declared(text: &str) -> Declared {
        let mut sources = Sources::new();
        let id = sources.add("main.xtex", text.as_bytes().to_vec());
        declared_in(&sources, id)
    }

    #[test]
    fn an_external_bibliography_is_found_and_given_its_extension() {
        let d = declared("\\bibliography{refs}");
        assert_eq!(d.resources.len(), 1);
        assert_eq!(d.resources[0].name, "refs.bib");
    }

    #[test]
    fn several_resources_may_share_one_declaration() {
        let d = declared("\\bibliography{refs, extra.bib}");
        let names: Vec<_> = d.resources.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, ["refs.bib", "extra.bib"]);
    }

    #[test]
    fn addbibresource_is_not_comma_separated() {
        // biblatex takes one resource per call, so a comma is part of the name
        // rather than a separator.
        let d = declared("\\addbibresource{my,file.bib}");
        assert_eq!(d.resources.len(), 1);
        assert_eq!(d.resources[0].name, "my,file.bib");
    }

    #[test]
    fn entries_inside_the_document_are_found() {
        // 14 of 224 measured files hold their bibliography this way, with 501
        // entries between them.
        let d = declared(
            "\\begin{thebibliography}{9}\n\
             \\bibitem{funsearch} B. Romera-Paredes et al.\n\
             \\bibitem{alphaevolve} A. Novikov et al.\n\
             \\end{thebibliography}",
        );
        assert_eq!(
            d.inline_keys.iter().map(String::as_str).collect::<Vec<_>>(),
            ["alphaevolve", "funsearch"]
        );
    }

    #[test]
    fn a_bibitem_with_a_label_is_found_too() {
        // 18 of the 501 measured entries use this form. Handling only the
        // common one loses their keys silently.
        let d = declared("\\bibitem[Knuth 1984]{knuth1984} D. Knuth.");
        assert_eq!(
            d.inline_keys.iter().map(String::as_str).collect::<Vec<_>>(),
            ["knuth1984"]
        );
    }

    #[test]
    fn a_declaration_inside_a_comment_declares_nothing() {
        let d = declared("% \\bibliography{never}\n\\bibliography{real}");
        let names: Vec<_> = d.resources.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, ["real.bib"]);
    }

    #[test]
    fn the_word_inside_verb_declares_nothing() {
        // 10 occurrences in the measured corpus are `\verb+\bibitem+` written
        // in prose about BibTeX, in 4 files that declare no bibliography. The
        // exclusion rule is what keeps those files silent.
        let d = declared("Always use \\verb+\\bibitem+ and \\verb|\\bibliography{x}| here.");
        assert!(d.inline_keys.is_empty());
        assert!(d.resources.is_empty());
    }

    #[test]
    fn a_computed_path_makes_the_whole_document_unavailable() {
        let d = declared("\\bibliography{\\myrefs}");
        assert!(d.computed.is_some());
        let bib = assemble(&d, |_| Some(b"@article{x,}".to_vec()));
        assert!(matches!(
            bib,
            Bibliography::Unavailable(Unavailable::ComputedPath { .. })
        ));
    }

    #[test]
    fn an_unreadable_resource_never_yields_a_partial_key_set() {
        // The rule that matters: a partial set produces false missing-key
        // errors, and an author who is wrongly accused stops trusting the real
        // ones.
        let d = declared("\\bibliography{present, absent}");
        let bib = assemble(&d, |name| {
            (name == "present.bib").then(|| b"@article{here,}".to_vec())
        });
        assert!(matches!(
            bib,
            Bibliography::Unavailable(Unavailable::Unreadable { .. })
        ));
        assert!(!bib.is_missing("anything"), "nothing may be called missing");
    }

    #[test]
    fn a_document_with_no_bibliography_is_unavailable_not_empty() {
        let d = declared("no bibliography here");
        let bib = assemble(&d, |_| None);
        assert_eq!(bib, Bibliography::Unavailable(Unavailable::NoneDeclared));
        assert!(!bib.is_missing("knuth1984"));
    }

    #[test]
    fn an_invented_key_is_missing_when_the_bibliography_is_complete() {
        let d = declared("\\bibliography{refs}");
        let bib = assemble(&d, |_| {
            Some(b"@article{knuth1984, title = {Literate Programming}}".to_vec())
        });
        assert!(!bib.is_missing("knuth1984"));
        assert!(bib.is_missing("smith2019"));
    }

    #[test]
    fn inline_entries_alone_are_enough_to_be_complete() {
        let d = declared("\\bibitem{funsearch} text");
        let bib = assemble(&d, |_| None);
        assert!(!bib.is_missing("funsearch"));
        assert!(bib.is_missing("invented"));
    }

    #[test]
    fn bib_metadata_entries_declare_no_citation_key() {
        let keys = keys_in_bib(
            b"@string{acm = {ACM}}\n@comment{ignore me}\n@preamble{\"x\"}\n@article{real, title={T}}",
        )
        .unwrap();
        assert_eq!(
            keys.iter().map(String::as_str).collect::<Vec<_>>(),
            ["real"]
        );
    }

    #[test]
    fn a_bib_entry_may_use_parentheses() {
        let keys = keys_in_bib(b"@article(paren_style, title = {T})").unwrap();
        assert!(keys.contains("paren_style"));
    }

    #[test]
    fn validation_matches_bibtex_on_the_experiment_corpus() {
        let rejected = [
            include_bytes!(
                "../../../tests/experiments/bib-validator/corpus/bad-01-unclosed-field.bib"
            )
            .as_slice(),
            include_bytes!(
                "../../../tests/experiments/bib-validator/corpus/bad-02-unclosed-entry.bib"
            )
            .as_slice(),
            include_bytes!(
                "../../../tests/experiments/bib-validator/corpus/bad-03-missing-comma.bib"
            )
            .as_slice(),
            include_bytes!(
                "../../../tests/experiments/bib-validator/corpus/bad-04-unclosed-quote.bib"
            )
            .as_slice(),
            include_bytes!(
                "../../../tests/experiments/bib-validator/corpus/bad-08-preamble-unbalanced.bib"
            )
            .as_slice(),
            include_bytes!(
                "../../../tests/experiments/bib-validator/corpus/bad-09-string-unbalanced.bib"
            )
            .as_slice(),
            b"@article{k, title={A}year={2020}}",
        ];
        for bytes in rejected {
            assert!(keys_in_bib(bytes).is_none());
        }

        let accepted = [
            include_bytes!("../../../tests/experiments/bib-validator/corpus/bad-05-no-key.bib")
                .as_slice(),
            include_bytes!(
                "../../../tests/experiments/bib-validator/corpus/bad-06-stray-brace.bib"
            )
            .as_slice(),
            include_bytes!(
                "../../../tests/experiments/bib-validator/corpus/bad-07-undefined-string.bib"
            )
            .as_slice(),
            include_bytes!("../../../tests/experiments/bib-validator/corpus/ok-01-plain.bib")
                .as_slice(),
            include_bytes!("../../../tests/experiments/bib-validator/corpus/ok-02-quotes.bib")
                .as_slice(),
            include_bytes!("../../../tests/experiments/bib-validator/corpus/ok-03-strings.bib")
                .as_slice(),
            include_bytes!(
                "../../../tests/experiments/bib-validator/corpus/ok-04-preamble-hash.bib"
            )
            .as_slice(),
            include_bytes!(
                "../../../tests/experiments/bib-validator/corpus/ok-05-comment-and-junk.bib"
            )
            .as_slice(),
            include_bytes!(
                "../../../tests/experiments/bib-validator/corpus/ok-06-nested-braces.bib"
            )
            .as_slice(),
            include_bytes!("../../../tests/experiments/bib-validator/corpus/ok-07-concat.bib")
                .as_slice(),
            include_bytes!(
                "../../../tests/experiments/bib-validator/corpus/ok-08-comment-unbalanced.bib"
            )
            .as_slice(),
            include_bytes!(
                "../../../tests/experiments/bib-validator/corpus/ok-09-comment-wraps-an-entry.bib"
            )
            .as_slice(),
            b"@comment{\" is ordinary comment text}\n@book{k, title={T}}",
            b"@article{k\"x, title={T}, year=2020}",
        ];
        for bytes in accepted {
            assert!(keys_in_bib(bytes).is_some());
        }
    }

    #[test]
    fn a_comment_is_skipped_to_the_next_at_sign_the_way_bibtex_skips_it() {
        // Both halves were settled by running the files through BibTeX 0.99e.
        // It accepts an unbalanced brace inside `@comment`, because it never
        // reads the body; and an entry a writer commented out by wrapping it
        // still resolves when cited, because the skip stops at the next `@`.
        // Validating the body would reject the first and lose the key in the
        // second, and losing a key turns a correct `@cite` into `XT1005`.
        let unbalanced = include_bytes!(
            "../../../tests/experiments/bib-validator/corpus/ok-08-comment-unbalanced.bib"
        );
        assert_eq!(
            keys_in_bib(unbalanced),
            Some(BTreeSet::from(["k1".to_owned()]))
        );

        let wrapped = include_bytes!(
            "../../../tests/experiments/bib-validator/corpus/ok-09-comment-wraps-an-entry.bib"
        );
        assert_eq!(
            keys_in_bib(wrapped),
            Some(BTreeSet::from([
                "commented_out".to_owned(),
                "k1".to_owned()
            ]))
        );
    }

    #[test]
    fn an_invalid_file_makes_the_whole_bibliography_unavailable() {
        let d = declared("\\bibliography{refs}");
        let bibliography = assemble(&d, |_| Some(b"@book{k,\n title = {never closed\n".to_vec()));
        assert_eq!(
            bibliography,
            Bibliography::Unavailable(Unavailable::UnparsableEntry {
                name: "refs.bib".to_owned(),
                detail: "a value opened at line 2 is never closed".to_owned(),
            })
        );
        assert!(!bibliography.is_missing("anything"));

        let mut sources = Sources::new();
        let id = sources.add("main.xtex", b"@cite(anything)".to_vec());
        let document = crate::parse(&sources, id);
        let mut table = SymbolTable::new();
        table.merge(&sources, &document);
        assert!(
            crate::check::check(&table, &bibliography)
                .iter()
                .all(|diagnostic| diagnostic.code != "XT1005")
        );
    }
    /// The three outcomes issue #12 requires, driven through the real pipeline:
    /// parse, build the symbol table, assemble the bibliography, compare.
    fn cited(document: &str, files: &[(&str, &str)]) -> Vec<String> {
        let mut sources = Sources::new();
        let id = sources.add("main.xtex", document.as_bytes().to_vec());
        let parsed = crate::parse(&sources, id);

        let mut table = SymbolTable::new();
        table.merge(&sources, &parsed);

        let found: BTreeMap<String, Vec<u8>> = files
            .iter()
            .map(|(n, t)| ((*n).to_owned(), t.as_bytes().to_vec()))
            .collect();
        let bibliography = assemble_from(&declared_in(&sources, id), &found);

        missing_citations(&table, &bibliography)
            .map(|(key, _)| key.to_owned())
            .collect()
    }

    #[test]
    fn a_key_the_bibliography_holds_passes() {
        let missing = cited(
            "\\bibliography{refs} @cite(knuth1984)",
            &[("refs.bib", "@book{knuth1984, title = {The TeXbook}}")],
        );
        assert!(missing.is_empty(), "found {missing:?}");
    }

    #[test]
    fn a_key_absent_from_a_complete_bibliography_fails() {
        let missing = cited(
            "\\bibliography{refs} @cite(invented2026)",
            &[("refs.bib", "@book{knuth1984, title = {The TeXbook}}")],
        );
        assert_eq!(missing, ["invented2026"]);
    }

    #[test]
    fn a_bibliography_that_could_not_be_read_reports_nothing() {
        // The same invented key. The only difference is that the `.bib` is not
        // there, and that difference must silence the diagnostic.
        let missing = cited("\\bibliography{refs} @cite(invented2026)", &[]);
        assert!(
            missing.is_empty(),
            "an unread bibliography was turned into a missing key: {missing:?}"
        );
    }

    #[test]
    fn a_plain_latex_citation_is_never_reported() {
        // `\\cite` is the author's LaTeX, outside any ExactTeX construct. Only
        // `@cite` carries the guarantee, so only `@cite` is checked.
        let missing = cited(
            "\\bibliography{refs} \\cite{invented2026}",
            &[("refs.bib", "@book{knuth1984}")],
        );
        assert!(missing.is_empty());
    }
}
