//! Command shapes, transcribed rather than recalled.
//!
//! `docs/grammar.md` §8 excludes a command's arguments from recognition, which
//! requires knowing which commands take arguments and in what shape. LaTeX has
//! a declarative notation for exactly that — `xparse` argument specifications —
//! so the shapes are read from a specification instead of inferred from a
//! corpus.
//!
//! # Provenance
//!
//! The table below is transcribed from unified-latex's CTAN database,
//! `packages/unified-latex-ctan/package/latex2e/provides.ts`, fetched on
//! 2026-08-29. It is data, not judgement: no entry here was written from
//! memory, and a signature that is wrong is wrong at the source rather than in
//! a transcription.
//!
//! # The notation
//!
//! | Token | Argument |
//! |---|---|
//! | `m` | mandatory, a balanced `{…}` or one token |
//! | `o` | optional, a balanced `[…]` |
//! | `s` | an optional `*` |
//! | `+` | prefix meaning the argument may span paragraphs; it does not change the delimiters |
//! | `d()` `r()` | delimited by the given pair |
//!
//! So `\\includegraphics` is `s o o m`: an optional star, two optional bracketed
//! arguments, then one mandatory braced one.

/// One argument a command takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Argument {
    /// A balanced `{…}`, or a single token when no brace follows.
    Mandatory,
    /// A balanced `[…]`, present or absent.
    Optional,
    /// An optional `*`.
    Star,
    /// Delimited by an explicit pair, such as `d()`.
    Delimited(u8, u8),
}

/// The shape of one command's arguments, in order.
pub type Signature = Vec<Argument>;

/// Parses an `xparse` argument specification.
///
/// Returns `None` for a specification this parser does not model, which is
/// treated exactly like an unknown command rather than guessed at.
#[must_use]
pub fn parse_signature(spec: &str) -> Option<Signature> {
    let mut out = Vec::new();
    let mut chars = spec.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            // A space separates arguments; `+` marks one as able to span
            // paragraphs. Neither changes where an argument begins or ends.
            ' ' | '+' => {}
            'm' => out.push(Argument::Mandatory),
            'o' => out.push(Argument::Optional),
            's' => out.push(Argument::Star),
            'd' | 'r' => {
                let open = chars.next()? as u32;
                let close = chars.next()? as u32;
                out.push(Argument::Delimited(
                    u8::try_from(open).ok()?,
                    u8::try_from(close).ok()?,
                ));
            }
            _ => return None,
        }
    }
    Some(out)
}

/// Signature of `name`, if the built-in table has one.
#[must_use]
pub fn signature_of(name: &[u8]) -> Option<Signature> {
    let name = std::str::from_utf8(name).ok()?;
    LATEX2E
        .binary_search_by_key(&name, |(n, _)| *n)
        .ok()
        .and_then(|i| parse_signature(LATEX2E[i].1))
}

/// Whether the built-in table knows this command at all.
#[must_use]
pub fn is_known(name: &[u8]) -> bool {
    std::str::from_utf8(name).is_ok_and(|n| LATEX2E.binary_search_by_key(&n, |(k, _)| *k).is_ok())
}

/// Commands whose arguments the bibliography reader also inspects.
///
/// Their bytes stay opaque for transport; the argument is read only to learn
/// which `.bib` files the document declares. Without this, `@cite` cannot be
/// checked at all, because the declaration lives in LaTeX the compiler does not
/// model.
pub const BIBLIOGRAPHY_COMMANDS: &[&str] = &["bibliography", "addbibresource"];

/// `latex2e` command signatures, sorted by name for binary search.
static LATEX2E: &[(&str, &str)] = &[
    ("Alph", "m"),
    ("Roman", "m"),
    ("_", "m"),
    ("abstract", "m"),
    ("addcontentsline", "m m m"),
    ("addtocontents", "m m"),
    ("addtocounter", "m m"),
    ("addtolength", "m m"),
    ("alph", "m"),
    ("arabic", "m"),
    ("array", "o m"),
    ("bibitem", "o m"),
    ("bibliography", "m"),
    ("bibliographystyle", "m"),
    ("caption", "o m"),
    ("chapter", "s o m"),
    ("cite", "o m"),
    ("colorbox", "o m m"),
    ("contentsline", "m m m"),
    ("date", "o m"),
    ("definecolor", "m m m"),
    ("description", "o"),
    ("discretionary", "m m m"),
    ("documentclass", "o m"),
    ("emph", "m"),
    ("enlargethispage", "s"),
    ("ensuremath", "m"),
    ("enumerate", "o"),
    ("fbox", "m"),
    ("fcolorbox", "o m m"),
    ("figure", "o"),
    ("filecontents", "o m"),
    ("fnsymbol", "m"),
    ("footnote", "o m"),
    ("footnotemark", "o"),
    ("footnotetext", "o m"),
    ("frac", "m m"),
    ("frame", "m"),
    ("framebox", "o o m"),
    ("hphantom", "m"),
    ("hspace", "s m"),
    ("hyphenation", "m"),
    ("include", "m"),
    ("includegraphics", "s o o m"),
    ("includeonly", "m"),
    ("input", "m"),
    ("item", "o"),
    ("itemize", "o"),
    ("label", "o m"),
    ("linebreak", "o"),
    ("list", "m m"),
    ("makebox", "d() o o m"),
    ("marginpar", "o m"),
    ("mathbf", "m"),
    ("mathcal", "m"),
    ("mathit", "m"),
    ("mathnormal", "m"),
    ("mathrm", "m"),
    ("mathsf", "m"),
    ("mathtt", "m"),
    ("mbox", "m"),
    ("minipage", "o o o m"),
    ("multicolumn", "m m m"),
    ("newcommand", "s +m o +o +m"),
    ("newcounter", "m o"),
    ("newenvironment", "s m o o m m"),
    ("newfont", "m m"),
    ("newlength", "m"),
    ("newsavebox", "m"),
    ("newtheorem", "s m o m o"),
    ("nolinebreak", "o"),
    ("nopagebreak", "o"),
    ("pagebreak", "o"),
    ("pagecolor", "o m"),
    ("pagenumbering", "m"),
    ("pagestyle", "m"),
    ("paragraph", "s o m"),
    ("parbox", "o o o m m"),
    ("part", "s o m"),
    ("phantom", "m"),
    ("picture", "r() d()"),
    ("providecommand", "s +m o +o +m"),
    ("raisebox", "m o o m"),
    ("ref", "s m"),
    ("reflectbox", "m"),
    ("refstepcounter", "m"),
    ("renewcommand", "s +m o +o +m"),
    ("renewenvironment", "s m o o m m"),
    ("resizebox", "s m m m"),
    ("roman", "m"),
    ("rotatebox", "o m m"),
    ("rule", "o m m"),
    ("savebox", "m o o m"),
    ("sbox", "m m"),
    ("scalebox", "m o m"),
    ("section", "s o m"),
    ("setcounter", "m m"),
    ("setlength", "m m"),
    ("settodepth", "m m"),
    ("settoheight", "m m"),
    ("settowidth", "m m"),
    ("sqrt", "o m"),
    ("stackrel", "m m"),
    ("stepcounter", "m"),
    ("stretch", "m"),
    ("subparagraph", "s o m"),
    ("subsection", "s o m"),
    ("subsubsection", "s o m"),
    ("table", "o"),
    ("tabular", "o m"),
    ("textbf", "m"),
    ("textit", "m"),
    ("textmd", "m"),
    ("textnormal", "m"),
    ("textrm", "m"),
    ("textsc", "m"),
    ("textsf", "m"),
    ("textsl", "m"),
    ("texttt", "m"),
    ("textup", "m"),
    ("thanks", "m"),
    ("thebibliography", "m"),
    ("thispagestyle", "m"),
    ("trivlist", "o"),
    ("underline", "m"),
    ("uppercase", "m"),
    ("usecounter", "m"),
    ("usepackage", "o m"),
    ("value", "m"),
    ("vphantom", "m"),
    ("vspace", "s m"),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_table_is_sorted_so_binary_search_is_valid() {
        let mut previous = "";
        for (name, _) in LATEX2E {
            assert!(*name > previous, "{name} is out of order after {previous}");
            previous = name;
        }
    }

    #[test]
    fn every_signature_in_the_table_parses() {
        for (name, spec) in LATEX2E {
            assert!(
                parse_signature(spec).is_some(),
                "{name} has a signature this parser does not model: {spec:?}"
            );
        }
    }

    #[test]
    fn the_shapes_that_matter_are_what_the_source_says() {
        // Spot checks against the transcribed file. If a future refresh of the
        // table changes one of these, that is a real change in LaTeX's own
        // description and should be noticed rather than absorbed.
        assert_eq!(
            signature_of(b"includegraphics").unwrap(),
            [
                Argument::Star,
                Argument::Optional,
                Argument::Optional,
                Argument::Mandatory
            ]
        );
        assert_eq!(
            signature_of(b"section").unwrap(),
            [Argument::Star, Argument::Optional, Argument::Mandatory]
        );
        assert_eq!(
            signature_of(b"caption").unwrap(),
            [Argument::Optional, Argument::Mandatory]
        );
        // \label takes an optional argument, which is easy to assume it does not.
        assert_eq!(
            signature_of(b"label").unwrap(),
            [Argument::Optional, Argument::Mandatory]
        );
    }

    #[test]
    fn an_unknown_command_has_no_signature() {
        assert!(signature_of(b"unknowncmd").is_none());
        assert!(!is_known(b"unknowncmd"));
    }

    #[test]
    fn a_specification_this_parser_does_not_model_is_refused() {
        assert!(parse_signature("m !o m").is_none());
        assert_eq!(
            parse_signature("+m o").unwrap(),
            [Argument::Mandatory, Argument::Optional]
        );
    }
}
