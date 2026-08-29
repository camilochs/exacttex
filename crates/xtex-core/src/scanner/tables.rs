//! Every classification table the scanner consults, in one place.
//!
//! Each table is transcribed from a source named beside it, and this file
//! is where `grammar.md` §8's lists are checked against the code — the
//! defects the external corpus found lived in that seam, not in the loop.

use super::EntryToken;
use crate::signatures::Argument;

/// How many adjacent groups an unknown command may claim before the parser
/// stops rather than guess that the next one is prose.
///
/// `docs/grammar.md` §8 fixes this at sixteen and says what would change it: a
/// real command absent from the databases with more arguments, or a documented
/// collision after a shorter run.
pub(crate) const MAX_UNKNOWN_COMMAND_GROUPS: usize = 16;

/// Environments whose bodies are copied rather than read.
///
/// Seeded from what a shallow LaTeX parser that handles real papers already
/// skips — `TexSoup`'s list, which names three this project had missed:
/// `Verbatim` from `fancyvrb`, `verbatimtab`, and `listing`. Extended per
/// project by `xtex.toml`.
pub const DEFAULT_VERBATIM_ENVIRONMENTS: &[&str] = &[
    "verbatim",
    "verbatim*",
    "Verbatim",
    "verbatimtab",
    "listing",
    "lstlisting",
    "minted",
];

/// Environments whose bodies are display math.
///
/// Transcribed from amsmath 2.17z (`amsmath.dtx`, 2025-07-09): its top-level
/// display definitions are equation, gather, align, alignat, xalignat,
/// xxalignat, flalign, and multline, with starred definitions where listed
/// here. `aligned`, `gathered`, `split`, and `cases` are inner structures, so
/// an occurrence of one cannot close the outer display region.
pub(crate) const DISPLAY_MATH_ENVIRONMENTS: &[(&str, &[Argument])] = &[
    ("align", &[]),
    ("align*", &[]),
    ("alignat", &[Argument::Mandatory]),
    ("alignat*", &[Argument::Mandatory]),
    ("equation", &[]),
    ("equation*", &[]),
    ("flalign", &[]),
    ("flalign*", &[]),
    ("gather", &[]),
    ("gather*", &[]),
    ("multline", &[]),
    ("multline*", &[]),
    ("xalignat", &[Argument::Mandatory]),
    ("xalignat*", &[Argument::Mandatory]),
    ("xxalignat", &[Argument::Mandatory]),
];

/// Every `@`-keyword, longest first so that `@import` is tried before `@id`.
pub(crate) const AT_TOKENS: &[EntryToken] = &[
    EntryToken::Import,
    EntryToken::Note,
    EntryToken::Add,
    EntryToken::Del,
    EntryToken::Sub,
    EntryToken::Cite,
    EntryToken::Ref,
    EntryToken::Id,
];

/// The citation commands a construct may name, longest first.
///
/// A citation construct is a LaTeX citation command written with `@`, so this
/// is a list of real command names rather than a vocabulary of our own. The
/// kernel provides `cite`; `natbib` provides `citep` and `citet`; `biblatex`
/// provides `textcite` and `parencite`. `docs/grammar.md` §4.
///
/// All five map to one [`EntryToken::Cite`]. Which command was written is in
/// the construct's own bytes, and the emitter reads it there — a variant per
/// command would put the same information in two places.
pub const DEFAULT_CITE_COMMANDS: &[&str] = &["parencite", "textcite", "citep", "citet", "cite"];

/// Commands whose argument is a *definition* rather than content.
///
/// The distinction §8 is actually about. A `\label` inside `\caption{…}` is a
/// real declaration — that argument is typeset. The same `\label` inside a
/// `\newcommand` body is not: nothing there has happened yet, and it may never
/// happen.
pub const DEFINITION_COMMANDS: &[&str] = &[
    "newcommand",
    "renewcommand",
    "providecommand",
    "newenvironment",
    "renewenvironment",
    "def",
    "edef",
    "gdef",
    "xdef",
    "csname",
];

/// The offset just past a command at `at` and every argument it claims.
///
/// For a command with a known signature, the arguments are exactly the ones the
/// signature selects. For one with no signature, §8 allows a run of adjacent
/// balanced groups to be treated as its arguments, bounded at sixteen — beyond
/// that the parser stops rather than assume the seventeenth group is prose.
/// Bytes that never begin a single-token mandatory argument.
///
/// A mandatory argument may be one token — `\\newcommand\\foo{}` is real LaTeX.
/// But these bytes open some *other* xparse argument form, so meeting one where
/// a mandatory argument was expected means the signature does not describe this
/// call. See `docs/grammar.md` §8.
pub(crate) const ARGUMENT_OPENERS: &[u8] = b"[<(*";

/// Known commands whose final mandatory argument is prose.
///
/// A caption is a sentence; treating it as data made `@ref(fig:x)` inside
/// one emit literally into the PDF with exit 0 — Phase 0a's gap 3, decided
/// as issue #83. Each entry is transcribed against its signature in the
/// table above; a command absent from the signature table cannot be listed
/// (`title` and `author` are not, and stay excluded — the conservative
/// default the issue fixes for the unclassified).
///
/// `item` is not here because its prose lives in its *optional* argument;
/// see [`TEXT_OPTIONAL_COMMANDS`].
pub(crate) const TEXT_MANDATORY_COMMANDS: &[&[u8]] = &[
    b"caption",
    b"chapter",
    b"emph",
    b"footnote",
    b"footnotetext",
    b"mbox",
    b"paragraph",
    b"part",
    b"section",
    b"subparagraph",
    b"subsection",
    b"subsubsection",
    b"textbf",
    b"textit",
    b"underline",
];
// `\texttt` is deliberately absent, against the issue's first list: it is
// the code font, and `\texttt{@ref(x)}` is how prose *shows* the literal
// token — fixture revisions/04 exists to hold exactly that. Converting it
// would corrupt any document that documents ExactTeX.

/// Known commands whose optional argument is prose.
pub(crate) const TEXT_OPTIONAL_COMMANDS: &[&[u8]] = &[b"item"];

/// Control words beginning with `if` that are not conditionals.
///
/// The name is everything after `\if`, and the comparison is against the
/// complete name: `\iffalse` is a real kernel conditional whose name also
/// begins with `f`, so a prefix match is exactly the wrong implementation.
/// One rule, called from both the opening site and the nested-count site, so
/// the two cannot drift apart.
///
/// - `thenelse` — `\ifthenelse` takes braced arguments and has no `\fi`; the
///   signature path handles it as a command.
/// - `f` — `\iff` is the kernel's ⟺ symbol, transcribed from `fontmath.ltx`
///   (TeX Live 2026-03-01): `\DeclareRobustCommand \iff{\;\Longleftrightarrow\;}`.
///   No `\fi` exists. Found quarantining real papers by the external corpus
///   (issue #79); package-defined braced conditionals such as `etoolbox`'s
///   `\ifblank` are deliberately not listed — extending this by configuration
///   is a policy decision recorded in the issue, not made here.
pub(crate) fn if_name_is_not_a_conditional(name: &[u8]) -> bool {
    matches!(name, b"thenelse" | b"f")
}
