//! The mechanical ramp from a LaTeX project to an ExactTeX one.
//!
//! `xtex adopt` rewrites, in live text only, the constructs the corpus
//! experiment measured as safe to rewrite by hand — citations, labels,
//! references, and the root's own `\input` edges — and then holds the
//! result to one guarantee: the emitter run over the converted file returns
//! the original bytes. The only admitted difference is the `.tex` extension
//! an `@import` writes back onto an `\input` that had none. A file whose
//! emission differs anywhere else is left as it was and the report says
//! where.
//!
//! Live text is the scanner's word, not this module's. Every region in
//! which an entry token is ordinary bytes (`docs/grammar.md` §8) is a region
//! in which nothing here is rewritten, because a construct written there
//! would ride into the PDF as literal text. One scanner decides both.
//!
//! Nothing here touches a filesystem. The host receives the converted
//! bytes and the report and decides where they go.

use std::collections::BTreeMap;

use crate::io::{IoError, SourceLoader};
use crate::scanner::{self, DEFAULT_CITE_COMMANDS, Piece};
use crate::source::{SourceId, Sources, Span};
use crate::{RevisionView, emit_view, parse};

/// Why a construct that stood in live text was left as written.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Reason {
    /// A star or an optional argument changes what the citation emits, and
    /// the construct carries neither.
    CitationOption,
    /// A key falls outside the grammar's `bibkey`, or the list carries
    /// whitespace the emitter would not write back.
    KeyOutsideGrammar,
    /// The name falls outside the grammar's `ident`.
    IdentOutsideGrammar,
    /// A starred or optional-argument `\label` or `\ref`.
    ReferenceOption,
    /// `\include` is never converted: its page-clearing and `.aux` behaviour
    /// are not what `@import` emits.
    Include,
    /// An `\input` inside an imported file. Only the root's edges are
    /// converted.
    NestedInput,
    /// The path carries `\`, `#`, `$`, a byte the import construct cannot
    /// hold literally, or edge spaces the emitter would drop.
    PathNotLiteral,
    /// No `.tex` file exists at the path beside the root.
    TargetAbsent,
    /// The target file exists but did not pass the guarantee itself.
    TargetLeft,
}

impl Reason {
    /// The sentence the report prints.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::CitationOption => "a star or an optional argument keeps the citation as written",
            Self::KeyOutsideGrammar => {
                "a key is outside the grammar's bibkey, or the list carries spaces"
            }
            Self::IdentOutsideGrammar => "the identifier is outside the grammar's ident",
            Self::ReferenceOption => "a star or an optional argument keeps the command as written",
            Self::Include => "\\include is never converted",
            Self::NestedInput => "\\input is converted in the root file only",
            Self::PathNotLiteral => {
                "the path is not literal: \\, #, $, a quote, a parenthesis or edge spaces"
            }
            Self::TargetAbsent => "no .tex file exists at that path beside the root",
            Self::TargetLeft => "the target file did not pass the guarantee",
        }
    }
}

/// One construct left as written, and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Left {
    /// The construct's bytes in the original file.
    pub span: Span,
    /// Why it was left.
    pub reason: Reason,
}

/// What one file's conversion rewrote.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Counts {
    /// Citation commands rewritten.
    pub citations: usize,
    /// Keys inside those commands.
    pub citation_keys: usize,
    /// `\label` commands rewritten to `@id`.
    pub ids: usize,
    /// `\ref` commands rewritten to `@ref`.
    pub refs: usize,
    /// `\input` commands rewritten to `@import`.
    pub imports: usize,
}

/// An `\input` the root rewrote to an `@import`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Import {
    /// The `\input{…}` bytes in the original file.
    pub span: Span,
    /// The path as the author wrote it, with or without `.tex`.
    pub path: String,
}

/// Whether a file is the root of the project or one the root imports.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    /// The file named on the command line. Its `\input` edges are converted.
    Root,
    /// A file the root imports. Its `\input` edges are left as written.
    Imported,
}

/// One file rewritten, before the guarantee is checked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Conversion {
    /// The converted bytes.
    pub bytes: Vec<u8>,
    /// What was rewritten.
    pub counts: Counts,
    /// What was left, and why.
    pub left: Vec<Left>,
    /// The imports rewritten, root only.
    pub imports: Vec<Import>,
}

/// Why a whole file was left as it was.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Failure {
    /// The file already carries an ExactTeX construct, so its emission is
    /// not its own bytes and the guarantee cannot be stated.
    AlreadyAnnotated,
    /// Emitting the converted file did not return the original bytes. The
    /// offset is the first byte that differs.
    Emission {
        /// First differing byte offset, in the original file.
        offset: usize,
    },
    /// The file passed, and the root did not; nothing is written for a
    /// project whose root is left.
    RootLeft,
}

impl std::fmt::Display for Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::AlreadyAnnotated => {
                write!(f, "the file already carries an ExactTeX construct")
            }
            Self::Emission { offset } => write!(
                f,
                "the emitted LaTeX differs from the original at byte {offset}"
            ),
            Self::RootLeft => write!(f, "the root was left, so nothing is written"),
        }
    }
}

/// One file's outcome.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileOutcome {
    /// The source, as the loader named it.
    pub source: SourceId,
    /// The file's name.
    pub name: String,
    /// The `.xtex` name the converted bytes belong under.
    pub output: String,
    /// The converted bytes, when the file passed.
    pub converted: Option<Vec<u8>>,
    /// What the conversion rewrote, or attempted to on a file that failed.
    pub counts: Counts,
    /// What the conversion left, and why.
    pub left: Vec<Left>,
    /// Why the file was left as it was, when it was.
    pub failure: Option<Failure>,
}

/// A whole project through the ramp.
#[derive(Debug)]
pub struct Adopted {
    /// Every source read, so a report can name lines.
    pub sources: Sources,
    /// The root first, then each file it imports, in the root's order.
    pub files: Vec<FileOutcome>,
}

impl Adopted {
    /// Whether every file passed.
    #[must_use]
    pub fn all_passed(&self) -> bool {
        self.files.iter().all(|file| file.failure.is_none())
    }
}

/// The path an `\input` names, as the file it loads.
#[must_use]
pub fn target_of(path: &str) -> String {
    // The exact suffix, as TeX reads it: `p.tex` is a file, `p.TEX` is not
    // the same file on the systems the corpus was measured on.
    if path.strip_suffix(".tex").is_some() {
        path.to_owned()
    } else {
        format!("{path}.tex")
    }
}

/// The `.xtex` name beside a `.tex` name.
#[must_use]
pub fn output_name(name: &str) -> String {
    name.strip_suffix(".tex")
        .map_or_else(|| format!("{name}.xtex"), |stem| format!("{stem}.xtex"))
}

/// Runs the ramp over one root and everything its `\input` reaches.
///
/// Children are converted and checked before the root's second pass, so
/// that an `\input` whose target was left stays an `\input`. A root that
/// fails the guarantee leaves the whole project as it was: with the root
/// untouched, a renamed child would break the author's build.
///
/// # Errors
///
/// Returns [`IoError`] when the root is not a `.tex` file, cannot be read,
/// or an import that existed a moment ago cannot be read.
pub fn adopt(loader: &impl SourceLoader, root: &str) -> Result<Adopted, IoError> {
    if root.strip_suffix(".tex").is_none() {
        return Err(IoError::Unresolvable {
            name: root.to_owned(),
            detail: "adopt reads a .tex file".to_owned(),
        });
    }
    let mut sources = Sources::new();
    let root_id = loader.load(root, None, &mut sources)?;
    let root_name = sources
        .get(root_id)
        .map_or_else(|| root.to_owned(), |source| source.name().to_owned());
    let root_bytes = sources
        .get(root_id)
        .map_or_else(Vec::new, |source| source.bytes().to_vec());

    if already_annotated(&root_bytes) {
        return Ok(Adopted {
            sources,
            files: vec![left_outcome(root_id, &root_name, Failure::AlreadyAnnotated)],
        });
    }

    // Pass one discovers the edges; existence is the only question asked.
    let discovered = convert(&root_bytes, Role::Root, &|path| {
        (!loader.file_exists(&root_name, &target_of(path))).then_some(Reason::TargetAbsent)
    });

    let mut children: Vec<FileOutcome> = Vec::new();
    let mut status: BTreeMap<String, Option<Reason>> = BTreeMap::new();
    for import in &discovered.imports {
        let target = target_of(&import.path);
        if status.contains_key(&target) {
            continue;
        }
        let child_id = match loader.load(&target, Some(root_id), &mut sources) {
            Ok(id) => id,
            Err(IoError::NotFound { .. }) => {
                status.insert(target, Some(Reason::TargetAbsent));
                continue;
            }
            Err(error) => return Err(error),
        };
        if child_id == root_id
            || sources
                .get(child_id)
                .is_some_and(|child| child.name() == root_name)
        {
            status.insert(target, None);
            continue;
        }
        let outcome = convert_file(&mut sources, child_id, Role::Imported, &|_| None);
        status.insert(target, outcome.failure.as_ref().map(|_| Reason::TargetLeft));
        children.push(outcome);
    }

    // Pass two rewrites only the edges whose targets passed.
    let root_outcome = convert_file(&mut sources, root_id, Role::Root, &|path| {
        status
            .get(&target_of(path))
            .copied()
            .unwrap_or(Some(Reason::TargetAbsent))
    });

    let root_left = root_outcome.failure.is_some();
    let mut files = vec![root_outcome];
    for mut child in children {
        if root_left && child.failure.is_none() {
            child.converted = None;
            child.failure = Some(Failure::RootLeft);
        }
        files.push(child);
    }
    Ok(Adopted { sources, files })
}

fn left_outcome(source: SourceId, name: &str, failure: Failure) -> FileOutcome {
    FileOutcome {
        source,
        name: name.to_owned(),
        output: output_name(name),
        converted: None,
        counts: Counts::default(),
        left: Vec::new(),
        failure: Some(failure),
    }
}

/// Converts one loaded source and checks the guarantee on it.
fn convert_file(
    sources: &mut Sources,
    id: SourceId,
    role: Role,
    import_left: &dyn Fn(&str) -> Option<Reason>,
) -> FileOutcome {
    let (name, bytes) = sources.get(id).map_or_else(
        || (String::new(), Vec::new()),
        |source| (source.name().to_owned(), source.bytes().to_vec()),
    );
    if already_annotated(&bytes) {
        return left_outcome(id, &name, Failure::AlreadyAnnotated);
    }
    let conversion = convert(&bytes, role, import_left);
    let output = output_name(&name);
    let failure = gate(&output, &bytes, &conversion).err();
    FileOutcome {
        source: id,
        name,
        output,
        converted: failure.is_none().then_some(conversion.bytes),
        counts: conversion.counts,
        left: conversion.left,
        failure,
    }
}

/// Whether the scanner already recognises a construct in these bytes.
fn already_annotated(bytes: &[u8]) -> bool {
    scanner::scan(bytes)
        .iter()
        .any(|piece| matches!(piece, Piece::Construct { .. } | Piece::Malformed { .. }))
}

/// The guarantee, as a check: emitting the conversion returns the original.
///
/// The expected bytes are the original with each rewritten `\input{p}`
/// carrying the `.tex` the emitter writes back; nothing else may differ.
///
/// # Errors
///
/// Returns [`Failure::Emission`] at the first byte that differs.
pub fn gate(output: &str, original: &[u8], conversion: &Conversion) -> Result<(), Failure> {
    let mut expected = Vec::with_capacity(original.len());
    let mut cursor = 0usize;
    for import in &conversion.imports {
        expected.extend_from_slice(&original[cursor..import.span.start()]);
        expected.extend_from_slice(b"\\input{");
        expected.extend_from_slice(target_of(&import.path).as_bytes());
        expected.push(b'}');
        cursor = import.span.end();
    }
    expected.extend_from_slice(&original[cursor..]);

    let mut sources = Sources::new();
    let id = sources.add(output, conversion.bytes.clone());
    let document = parse(&sources, id);
    let mut emitted = Vec::new();
    if emit_view(&sources, &document, RevisionView::Original, &mut emitted).is_err() {
        return Err(Failure::Emission { offset: 0 });
    }
    if emitted == expected {
        return Ok(());
    }
    let offset = emitted
        .iter()
        .zip(&expected)
        .position(|(a, b)| a != b)
        .unwrap_or_else(|| emitted.len().min(expected.len()));
    Err(Failure::Emission { offset })
}

/// Rewrites one file's live text.
///
/// `import_left` answers, for an `\input` path as written, why it must stay
/// an `\input` — or `None` to rewrite it. Only a [`Role::Root`] file asks.
#[must_use]
pub fn convert(
    bytes: &[u8],
    role: Role,
    import_left: &dyn Fn(&str) -> Option<Reason>,
) -> Conversion {
    let mut conversion = Conversion {
        bytes: Vec::with_capacity(bytes.len()),
        counts: Counts::default(),
        left: Vec::new(),
        imports: Vec::new(),
    };
    let mut cursor = 0usize;
    for piece in scanner::scan(bytes) {
        let (span, rewrite) = match piece {
            Piece::Arguments(span) => (
                span,
                rewrite_command(bytes, span, role, import_left, &mut conversion),
            ),
            Piece::Excluded(span) => (span, rewrite_header_label(bytes, span, &mut conversion)),
            _ => continue,
        };
        let Some((end, rewrite)) = rewrite else {
            continue;
        };
        conversion
            .bytes
            .extend_from_slice(&bytes[cursor..span.start()]);
        conversion.bytes.extend_from_slice(&rewrite);
        cursor = end;
    }
    conversion.bytes.extend_from_slice(&bytes[cursor..]);
    conversion
}

/// The control word a piece begins with, and the bytes after it.
fn control_word(region: &[u8]) -> Option<(&[u8], &[u8])> {
    let rest = region.strip_prefix(b"\\")?;
    let end = rest
        .iter()
        .take_while(|byte| byte.is_ascii_alphabetic())
        .count();
    (end > 0).then(|| (&rest[..end], &rest[end..]))
}

/// The interior of `rest` when it is exactly one braced group.
fn sole_group(rest: &[u8]) -> Option<&[u8]> {
    (scanner::balanced_end(rest, 0) == Some(rest.len())).then(|| &rest[1..rest.len() - 1])
}

fn is_ident(bytes: &[u8]) -> bool {
    let mut iter = bytes.iter();
    iter.next().is_some_and(u8::is_ascii_alphabetic)
        && iter
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b':' | b'.' | b'-'))
}

fn is_bibkey(bytes: &[u8]) -> bool {
    let mut iter = bytes.iter();
    iter.next().is_some_and(u8::is_ascii_alphanumeric)
        && iter.all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b':' | b'.' | b'+' | b'/' | b'-')
        })
}

/// Whether a key list is one the emitter writes back byte for byte: keys
/// inside the grammar, separated by bare commas.
fn keys_in_grammar(keys: &[u8]) -> bool {
    keys.split(|byte| *byte == b',').all(is_bibkey)
}

/// A rewrite for a command piece, with the offset conversion resumes at.
fn rewrite_command(
    bytes: &[u8],
    span: Span,
    role: Role,
    import_left: &dyn Fn(&str) -> Option<Reason>,
    conversion: &mut Conversion,
) -> Option<(usize, Vec<u8>)> {
    let region = &bytes[span.start()..span.end()];
    let (name, rest) = control_word(region)?;
    let leave = |conversion: &mut Conversion, reason| {
        conversion.left.push(Left { span, reason });
        None
    };
    // A signature the call refuted leaves the star and the groups after
    // it outside the piece; the report names the whole call.
    let leave_call = |conversion: &mut Conversion, reason| {
        let end = option_tail(bytes, span.end());
        conversion.left.push(Left {
            span: Span::new(
                u32::try_from(span.start()).unwrap_or(u32::MAX),
                u32::try_from(end).unwrap_or(u32::MAX),
            ),
            reason,
        });
        None
    };
    if DEFAULT_CITE_COMMANDS
        .iter()
        .any(|command| command.as_bytes() == name)
    {
        return match sole_group(rest) {
            Some(keys) if keys_in_grammar(keys) => {
                conversion.counts.citations += 1;
                conversion.counts.citation_keys += keys.split(|byte| *byte == b',').count();
                Some((span.end(), at_construct(name, keys)))
            }
            Some(_) => leave(conversion, Reason::KeyOutsideGrammar),
            // A signature that did not fit leaves the star or bracket just
            // past the piece; either way the command carries an option.
            None if rest.first().is_some_and(|b| matches!(b, b'*' | b'[')) => {
                leave(conversion, Reason::CitationOption)
            }
            None if rest.is_empty()
                && bytes
                    .get(span.end())
                    .is_some_and(|b| matches!(b, b'*' | b'[')) =>
            {
                leave_call(conversion, Reason::CitationOption)
            }
            None => None,
        };
    }
    match name {
        b"label" | b"ref" => match sole_group(rest) {
            Some(ident) if is_ident(ident) => {
                let keyword: &[u8] = if name == b"label" {
                    conversion.counts.ids += 1;
                    b"id"
                } else {
                    conversion.counts.refs += 1;
                    b"ref"
                };
                Some((span.end(), at_construct(keyword, ident)))
            }
            Some(_) => leave(conversion, Reason::IdentOutsideGrammar),
            None if rest.first().is_some_and(|b| matches!(b, b'*' | b'[')) => {
                leave(conversion, Reason::ReferenceOption)
            }
            None => None,
        },
        b"include" => sole_group(rest).and_then(|_| leave(conversion, Reason::Include)),
        b"input" => {
            let path = sole_group(rest)?;
            if role == Role::Imported {
                return leave(conversion, Reason::NestedInput);
            }
            let Some(path) = literal_path(path) else {
                return leave(conversion, Reason::PathNotLiteral);
            };
            if let Some(reason) = import_left(path) {
                return leave(conversion, reason);
            }
            conversion.counts.imports += 1;
            conversion.imports.push(Import {
                span,
                path: path.to_owned(),
            });
            let mut out = Vec::with_capacity(path.len() + 12);
            out.extend_from_slice(b"@import(\"");
            out.extend_from_slice(path.strip_suffix(".tex").unwrap_or(path).as_bytes());
            out.extend_from_slice(b".xtex\")");
            Some((span.end(), out))
        }
        _ => None,
    }
}

/// The end of the star and the bracketed and braced groups that follow a
/// command at `from`, for the report only.
fn option_tail(bytes: &[u8], from: usize) -> usize {
    let mut at = from;
    if bytes.get(at) == Some(&b'*') {
        at += 1;
    }
    loop {
        match bytes.get(at) {
            Some(b'[') => match scanner::optional_argument_end(bytes, at) {
                Some(end) => at = end,
                None => return at,
            },
            Some(b'{') => match scanner::balanced_end(bytes, at) {
                Some(end) => at = end,
                None => return at,
            },
            _ => return at,
        }
    }
}

/// A path the import construct carries as written: UTF-8, no `\`, `#`, `$`,
/// no byte that would end the construct early, and no edge whitespace the
/// emitter would drop.
fn literal_path(path: &[u8]) -> Option<&str> {
    let path = std::str::from_utf8(path).ok()?;
    let literal = !path.is_empty()
        && path.trim() == path
        && !path.contains(['\\', '#', '$', '"', ')', '{', '}'])
        && !path.chars().any(char::is_control);
    literal.then_some(path)
}

fn at_construct(keyword: &[u8], payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(keyword.len() + payload.len() + 3);
    out.push(b'@');
    out.extend_from_slice(keyword);
    out.push(b'(');
    out.extend_from_slice(payload);
    out.push(b')');
    out
}

/// A `\label` in a display-math environment's header slot.
///
/// The scanner's excluded body for such an environment begins after the
/// header slot's whitespace (`docs/grammar.md` §4), so a `\label` at the
/// body's first byte is the label the slot admits. Deeper labels stay:
/// math is an exclusion region, and the slot is its one opening.
fn rewrite_header_label(
    bytes: &[u8],
    span: Span,
    conversion: &mut Conversion,
) -> Option<(usize, Vec<u8>)> {
    let region = &bytes[span.start()..span.end()];
    if region.starts_with(b"\\[")
        || region.starts_with(b"$$")
        || !scanner::is_display_math_region(region)
    {
        return None;
    }
    let rest = region.strip_prefix(b"\\label")?;
    let end = scanner::balanced_end(rest, 0)?;
    let ident = &rest[1..end - 1];
    let label = Span::new(
        u32::try_from(span.start()).unwrap_or(u32::MAX),
        u32::try_from(span.start() + b"\\label".len() + end).unwrap_or(u32::MAX),
    );
    if !is_ident(ident) {
        conversion.left.push(Left {
            span: label,
            reason: Reason::IdentOutsideGrammar,
        });
        return None;
    }
    conversion.counts.ids += 1;
    Some((label.end(), at_construct(b"id", ident)))
}

/// One-based line and column of an offset.
fn line_column(bytes: &[u8], offset: usize) -> (usize, usize) {
    let before = &bytes[..offset.min(bytes.len())];
    // `docs/decisions/0005`: no dependency for a count taken once per line
    // of the report.
    #[allow(clippy::naive_bytecount)]
    let line = before.iter().filter(|byte| **byte == b'\n').count() + 1;
    let start = before
        .iter()
        .rposition(|byte| *byte == b'\n')
        .map_or(0, |at| at + 1);
    (line, offset - start + 1)
}

/// Renders the report as JSON — one renderer for the CLI and the module.
pub fn to_json(adopted: &Adopted, out: &mut String) {
    use std::fmt::Write as _;
    out.push_str("{\"root\":");
    crate::json::write_text(
        adopted.files.first().map_or("", |file| file.name.as_str()),
        out,
    );
    out.push_str(",\"files\":[");
    for (index, file) in adopted.files.iter().enumerate() {
        if index > 0 {
            out.push(',');
        }
        let bytes = adopted
            .sources
            .get(file.source)
            .map_or(&[][..], |source| source.bytes());
        out.push_str("{\"file\":");
        crate::json::write_text(&file.name, out);
        out.push_str(",\"output\":");
        crate::json::write_text(&file.output, out);
        let _ = write!(
            out,
            ",\"converted\":{},\"citations\":{},\"citation_keys\":{},\"ids\":{},\"refs\":{},\"imports\":{}",
            file.failure.is_none(),
            file.counts.citations,
            file.counts.citation_keys,
            file.counts.ids,
            file.counts.refs,
            file.counts.imports
        );
        if let Some(failure) = &file.failure {
            out.push_str(",\"failure\":");
            crate::json::write_text(&failure.to_string(), out);
        }
        out.push_str(",\"left\":[");
        for (index, left) in file.left.iter().enumerate() {
            if index > 0 {
                out.push(',');
            }
            let (line, column) = line_column(bytes, left.span.start());
            let _ = write!(out, "{{\"line\":{line},\"column\":{column},\"construct\":");
            let construct = bytes
                .get(left.span.start()..left.span.end())
                .unwrap_or_default();
            crate::json::write_text(&String::from_utf8_lossy(construct), out);
            out.push_str(",\"reason\":");
            crate::json::write_text(left.reason.as_str(), out);
            out.push('}');
        }
        out.push_str("]}");
    }
    out.push_str("]}");
}

/// Renders the report the way a person reads it.
pub fn render(adopted: &Adopted, out: &mut String) {
    use std::fmt::Write as _;
    for file in &adopted.files {
        let bytes = adopted
            .sources
            .get(file.source)
            .map_or(&[][..], |source| source.bytes());
        match &file.failure {
            None => {
                let counts = file.counts;
                let _ = writeln!(
                    out,
                    "{} -> {}: {} citations ({} keys), {} ids, {} refs, {} imports",
                    file.name,
                    file.output,
                    counts.citations,
                    counts.citation_keys,
                    counts.ids,
                    counts.refs,
                    counts.imports
                );
            }
            Some(failure) => {
                let _ = writeln!(out, "{}: left as it was — {failure}", file.name);
            }
        }
        for left in &file.left {
            let (line, column) = line_column(bytes, left.span.start());
            let construct = bytes
                .get(left.span.start()..left.span.end())
                .unwrap_or_default();
            let _ = writeln!(
                out,
                "  left: {}:{line}:{column} {} — {}",
                file.name,
                String::from_utf8_lossy(construct),
                left.reason.as_str()
            );
        }
    }
    let converted = adopted
        .files
        .iter()
        .filter(|file| file.failure.is_none())
        .count();
    let _ = writeln!(
        out,
        "{converted} of {} files converted",
        adopted.files.len()
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::io::Memory;

    fn plain(bytes: &[u8]) -> Conversion {
        convert(bytes, Role::Imported, &|_| None)
    }

    #[test]
    fn a_citation_with_bare_comma_keys_is_rewritten_and_one_with_a_space_is_not() {
        let converted = plain(b"See \\citep{a,b} and \\cite{c, d}.");
        assert_eq!(converted.bytes, b"See @citep(a,b) and \\cite{c, d}.");
        assert_eq!(converted.counts.citations, 1);
        assert_eq!(converted.counts.citation_keys, 2);
        assert_eq!(converted.left.len(), 1);
        assert_eq!(converted.left[0].reason, Reason::KeyOutsideGrammar);
    }

    #[test]
    fn a_starred_or_optional_citation_stays() {
        let converted = plain(b"\\cite*{a} \\cite[p.~3]{b} \\citep*{c} \\citet[see][]{d}");
        assert_eq!(
            converted.bytes,
            b"\\cite*{a} \\cite[p.~3]{b} \\citep*{c} \\citet[see][]{d}"
        );
        assert_eq!(converted.counts, Counts::default());
        assert!(
            converted
                .left
                .iter()
                .all(|left| left.reason == Reason::CitationOption),
            "{:?}",
            converted.left
        );
        assert_eq!(converted.left.len(), 4);
    }

    #[test]
    fn labels_and_refs_inside_the_grammar_are_rewritten() {
        let converted =
            plain(b"\\section{A}\\label{sec:a}\nSee \\ref{sec:a}, \\ref{bad name}, \\ref*{x}.");
        assert_eq!(
            converted.bytes,
            b"\\section{A}@id(sec:a)\nSee @ref(sec:a), \\ref{bad name}, \\ref*{x}."
        );
        assert_eq!(converted.counts.ids, 1);
        assert_eq!(converted.counts.refs, 1);
        let reasons: Vec<_> = converted.left.iter().map(|left| left.reason).collect();
        assert_eq!(
            reasons,
            [Reason::IdentOutsideGrammar, Reason::ReferenceOption]
        );
    }

    #[test]
    fn excluded_regions_are_not_rewritten() {
        let input = b"\\verb|\\label{a}| % \\label{b}\n\\begin{lstlisting}\n\\label{c}\n\\end{lstlisting}\n\\iffalse \\label{d} \\fi $\\label{e}$ \\newcommand{\\x}{\\ref{f}}";
        let converted = plain(input);
        assert_eq!(converted.bytes, input);
        assert_eq!(converted.counts, Counts::default());
    }

    #[test]
    fn the_display_math_header_slot_is_live_and_the_body_is_not() {
        let converted = plain(
            b"\\begin{equation}\n  \\label{eq:a}\n  x = y \\label{eq:b}\n\\end{equation}\n\\[ \\label{eq:c} \\]",
        );
        assert_eq!(
            converted.bytes,
            b"\\begin{equation}\n  @id(eq:a)\n  x = y \\label{eq:b}\n\\end{equation}\n\\[ \\label{eq:c} \\]"
        );
        assert_eq!(converted.counts.ids, 1);
    }

    #[test]
    fn the_root_rewrites_an_existing_input_and_leaves_the_rest() {
        let store = Memory::new()
            .with_input(
                "main.tex",
                b"\\input{a}\\input{b.tex}\\input{c}\\include{a}\\input{\\x}".as_slice(),
            )
            .with_input("a.tex", b"\\input{d}".as_slice())
            .with_input("b.tex", b"".as_slice())
            .with_input("d.tex", b"".as_slice());
        let adopted = adopt(&store, "main.tex").unwrap();
        assert!(adopted.all_passed(), "{:?}", adopted.files);
        assert_eq!(
            adopted.files[0].converted.as_deref(),
            Some(
                b"@import(\"a.xtex\")@import(\"b.xtex\")\\input{c}\\include{a}\\input{\\x}"
                    .as_slice()
            )
        );
        let reasons: Vec<_> = adopted.files[0]
            .left
            .iter()
            .map(|left| left.reason)
            .collect();
        assert_eq!(
            reasons,
            [
                Reason::TargetAbsent,
                Reason::Include,
                Reason::PathNotLiteral
            ]
        );
        // The imported file keeps its own \input.
        assert_eq!(adopted.files[1].name, "a.tex");
        assert_eq!(
            adopted.files[1].converted.as_deref(),
            Some(b"\\input{d}".as_slice())
        );
        assert_eq!(adopted.files[1].left[0].reason, Reason::NestedInput);
        assert_eq!(adopted.files.len(), 3);
    }

    #[test]
    fn the_gate_rejects_a_conversion_whose_emission_differs() {
        // A hand-made conversion the rules would never produce: the emitter
        // writes `\cite{a,b}` for it, and the original said `\cite{a, b}`.
        let original = b"\\cite{a, b}";
        let conversion = Conversion {
            bytes: b"@cite(a, b)".to_vec(),
            counts: Counts::default(),
            left: Vec::new(),
            imports: Vec::new(),
        };
        assert_eq!(
            gate("main.xtex", original, &conversion),
            Err(Failure::Emission { offset: 8 })
        );
    }

    #[test]
    fn a_file_already_carrying_a_construct_is_left() {
        let store = Memory::new().with_input("main.tex", b"@ref(x) \\cite{a}".as_slice());
        let adopted = adopt(&store, "main.tex").unwrap();
        assert_eq!(adopted.files[0].failure, Some(Failure::AlreadyAnnotated));
        assert!(adopted.files[0].converted.is_none());
    }

    #[test]
    fn a_child_that_fails_keeps_its_input_in_the_root() {
        let store = Memory::new()
            .with_input("main.tex", b"\\input{a}".as_slice())
            .with_input("a.tex", b"@id(x)".as_slice());
        let adopted = adopt(&store, "main.tex").unwrap();
        assert_eq!(
            adopted.files[0].converted.as_deref(),
            Some(b"\\input{a}".as_slice())
        );
        assert_eq!(adopted.files[0].left[0].reason, Reason::TargetLeft);
        assert_eq!(adopted.files[1].failure, Some(Failure::AlreadyAnnotated));
    }

    #[test]
    fn a_root_that_is_left_leaves_its_children_too() {
        // A root named under a directory emits `\input{paper/a.tex}`, which
        // is what `xtex build` writes from the project root and is not the
        // author's `\input{a}`: the gate refuses, and the child that passed
        // on its own is left with it.
        let store = Memory::new()
            .with_input("paper/main.tex", b"\\input{a}".as_slice())
            .with_input("paper/a.tex", b"\\cite{k}".as_slice());
        let adopted = adopt(&store, "paper/main.tex").unwrap();
        assert_eq!(
            adopted.files[0].failure,
            Some(Failure::Emission { offset: 7 })
        );
        assert_eq!(adopted.files[1].failure, Some(Failure::RootLeft));
        assert!(adopted.files[1].converted.is_none());

        let store = Memory::new()
            .with_input("main.tex", b"\\input{a}".as_slice())
            .with_input("a.tex", b"\\cite{k}".as_slice());
        let adopted = adopt(&store, "main.tex").unwrap();
        assert!(adopted.all_passed());
        assert_eq!(
            adopted.files[1].converted.as_deref(),
            Some(b"@cite(k)".as_slice())
        );
    }

    #[test]
    fn the_report_names_lines_and_reasons() {
        let store = Memory::new().with_input("main.tex", b"a\n\\cite[p]{k} \\label{x}".as_slice());
        let adopted = adopt(&store, "main.tex").unwrap();
        let mut json = String::new();
        to_json(&adopted, &mut json);
        assert_eq!(
            json,
            "{\"root\":\"main.tex\",\"files\":[{\"file\":\"main.tex\",\"output\":\"main.xtex\",\"converted\":true,\"citations\":0,\"citation_keys\":0,\"ids\":1,\"refs\":0,\"imports\":0,\"left\":[{\"line\":2,\"column\":1,\"construct\":\"\\\\cite[p]{k}\",\"reason\":\"a star or an optional argument keeps the citation as written\"}]}]}"
        );
        let mut text = String::new();
        render(&adopted, &mut text);
        assert_eq!(
            text,
            "main.tex -> main.xtex: 0 citations (0 keys), 1 ids, 0 refs, 0 imports\n  left: main.tex:2:1 \\cite[p]{k} — a star or an optional argument keeps the citation as written\n1 of 1 files converted\n"
        );
    }
}
