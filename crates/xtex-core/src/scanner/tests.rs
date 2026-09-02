use super::tables::{DISPLAY_MATH_ENVIRONMENTS, MAX_UNKNOWN_COMMAND_GROUPS};
use super::*;

fn reassemble(bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    for piece in scan(bytes) {
        let span = match piece {
            Piece::Text(s)
            | Piece::Excluded(s)
            | Piece::Arguments(s)
            | Piece::Quarantined(s)
            | Piece::Construct { span: s, .. }
            | Piece::Malformed { span: s, .. } => s,
        };
        out.extend_from_slice(&bytes[span.start()..span.end()]);
    }
    out
}

/// Where a piece sits, whatever kind it is.
fn extent(piece: &Piece) -> Span {
    match piece {
        Piece::Text(s)
        | Piece::Excluded(s)
        | Piece::Arguments(s)
        | Piece::Quarantined(s)
        | Piece::Construct { span: s, .. }
        | Piece::Malformed { span: s, .. } => *s,
    }
}

/// Inputs chosen so that truncating them enters every region the scanner
/// has, and leaves each one unterminated in turn.
const AWKWARD: &[&[u8]] = &[
    b"\\section{Caf\xE9}\r\n%% comment\t\n\\ref{a} @ref(b) trailing",
    b"before %comment\n@id(x) $math$ after",
    b"\\verb+@ref(a)+ then @ref(b)",
    b"\\begin{verbatim}\n@id(v)\n\\end{verbatim} @id(real)",
    b"\\begin{equation}\n@id(eq:x)\nX \\in [A,B)",
    b"\\makeatletter \\a@b \\makeatother @id(after)",
    b"$$@ref(display)$$ and \\[@ref(bracket)\\] done",
    b"latex { \\raw{@ref(inside)} } @ref(outside)",
    b"\\figure(f) { caption = {C} } @ref(f)",
    b"text \\unknowncommand{a}{b} more @cite(k)",
];

#[test]
fn the_pieces_cover_every_byte_exactly_once() {
    // The reassembly test above compares the concatenation, which hides
    // *where* coverage broke. This asserts the property directly: pieces
    // run in order, start where the last one ended, and reach the end.
    //
    // It exists because the same defect appeared twice — a region entered
    // without recording where it began, so a later piece restarted inside
    // an earlier one. Both times the bytes were emitted twice and both
    // times it took a fixture to notice.
    for input in AWKWARD {
        for cut in 0..=input.len() {
            let slice = &input[..cut];
            let mut next = 0usize;
            for piece in scan(slice) {
                let span = extent(&piece);
                assert_eq!(
                    span.start(),
                    next,
                    "piece {piece:?} does not start where the last one ended, \
                     in {slice:?} cut at {cut}"
                );
                assert!(
                    span.end() >= span.start(),
                    "piece {piece:?} ends before it starts"
                );
                next = span.end();
            }
            assert_eq!(
                next,
                slice.len(),
                "the pieces stop before the end of {slice:?} cut at {cut}"
            );
        }
    }
}

fn constructs(bytes: &[u8]) -> Vec<EntryToken> {
    let mut found = Vec::new();
    for piece in scan(bytes) {
        piece.walk(&mut |piece| {
            if let Piece::Construct { kind, .. } = piece {
                found.push(*kind);
            }
        });
    }
    found
}

#[test]
fn scanning_covers_every_byte_exactly_once() {
    for input in [
        b"plain text".as_slice(),
        b"@ref(x) and @id(y)",
        b"% @ref(hidden)\n@ref(seen)",
        b"$@ref(math)$ @ref(prose)",
        b"\\verb|@ref(v)| @ref(after)",
        b"latex {@ref(raw)} @ref(after)",
        b"\\makeatletter @ref(no) \\makeatother @ref(yes)",
        b"unterminated @ref(x",
        b"",
    ] {
        assert_eq!(reassemble(input), input, "lost bytes in {input:?}");
    }
}

#[test]
fn a_construct_in_prose_is_recognised() {
    assert_eq!(
        constructs(b"See @ref(a) and @id(b)."),
        [EntryToken::Ref, EntryToken::Id]
    );
}

#[test]
fn a_comment_hides_a_construct() {
    assert_eq!(constructs(b"% @ref(a)\n@ref(b)"), [EntryToken::Ref]);
}

#[test]
fn an_escaped_percent_does_not_open_a_comment() {
    // 120 of these appear inside captions in the corpus; treating them as
    // comments truncates real content.
    assert_eq!(constructs(b"94\\% then @ref(a)"), [EntryToken::Ref]);
    assert_eq!(constructs(b"a \\\\% then @ref(a)"), []);
}

#[test]
fn math_hides_a_construct_and_prose_after_it_does_not() {
    assert_eq!(constructs(b"$@ref(a)$ @ref(b)"), [EntryToken::Ref]);
    assert_eq!(constructs(b"$$@ref(a)$$ @ref(b)"), [EntryToken::Ref]);
    assert_eq!(constructs(b"\\[@ref(a)\\] @ref(b)"), [EntryToken::Ref]);
}

#[test]
fn display_math_environments_hide_commands_and_constructs() {
    // Every construct but `@id`: an equation is labelled from inside its
    // body, so the declaration is recognised there and nothing else is.
    for (name, signature) in DISPLAY_MATH_ENVIRONMENTS {
        let argument = if signature.is_empty() { "" } else { "{2}" };
        let input = format!(
            "\\begin{{{name}}}{argument} X \\in [A,B) @id(inside) @ref(hidden) \\bigg \\end{{{name}}} @ref(after)"
        );
        assert_eq!(
            constructs(input.as_bytes()),
            [EntryToken::Id, EntryToken::Ref],
            "{name} did not hide its body"
        );
        assert!(
            !scan(input.as_bytes())
                .iter()
                .any(|piece| matches!(piece, Piece::Quarantined(_))),
            "{name} quarantined a bounded display"
        );
    }
}

#[test]
fn a_display_body_opening_with_a_bracket_is_still_hidden_and_so_is_the_next_one() {
    // `\begin{equation} [a,b)` once claimed the bracket as an optional
    // argument of `\begin`, ran past the body's first byte, and left the
    // opening pending — so neither this body nor any later display in the
    // file was hidden (corpus E2, arXiv 2312.17141).
    for (name, signature) in DISPLAY_MATH_ENVIRONMENTS {
        let argument = if signature.is_empty() { "" } else { "{2}" };
        let input = format!(
            "\\begin{{{name}}}{argument} [A,B] @ref(hidden) \\end{{{name}}} then\n\\begin{{{name}}}{argument}\n@id(eq:b) @ref(hidden) \\end{{{name}}} @ref(after)"
        );
        assert_eq!(
            constructs(input.as_bytes()),
            [EntryToken::Id, EntryToken::Ref],
            "{name}: {input}"
        );
    }
}

#[test]
fn an_id_inside_a_display_body_is_a_declaration_and_the_rest_stays_formula() {
    for input in [
        &b"\\begin{equation}\n x = 1 @id(eq:a) @ref(no)\n\\end{equation} @ref(after)"[..],
        b"\\begin{align}\n y &= 2 @id(eq:a) \\\\ z &= 3 @cite(no)\n\\end{align} @ref(after)",
        b"\\[ z = 3 @id(eq:a) @ref(no) \\] @ref(after)",
        b"$$ z = 3 @id(eq:a) $$ @ref(after)",
    ] {
        assert_eq!(
            constructs(input),
            [EntryToken::Id, EntryToken::Ref],
            "{}",
            String::from_utf8_lossy(input)
        );
    }
    // Inline math and verbatim bodies recognise nothing, as before.
    assert_eq!(constructs(b"$ @id(no) $ @ref(after)"), [EntryToken::Ref]);
    assert_eq!(
        constructs(b"\\begin{verbatim}\n@id(no)\n\\end{verbatim} @ref(after)"),
        [EntryToken::Ref]
    );
}

#[test]
fn a_display_header_id_is_recognised_before_the_body() {
    let input = b"\\begin{equation}\n @id(eq:x)\n E = @ref(hidden) \\end{equation} @ref(after)";
    assert_eq!(constructs(input), [EntryToken::Id, EntryToken::Ref],);
}

#[test]
fn an_inner_math_environment_does_not_close_the_outer_region() {
    let input = b"\\begin{equation} X=\\begin{cases}a\\end{cases} @ref(hidden) \\
                  \\end{equation} @ref(after)";
    assert_eq!(constructs(input), [EntryToken::Ref]);
}

#[test]
fn an_unterminated_display_environment_quarantines_to_eof() {
    let input = b"\\begin{equation}\n X = @ref(hidden)";
    let pieces = scan(input);
    assert_eq!(constructs(input), []);
    assert!(matches!(pieces.last(), Some(Piece::Quarantined(span)) if span.end() == input.len()));
}

#[test]
fn an_unclosed_optional_bracket_is_prose_and_quarantines_nothing() {
    // The corpus reproducer: `\foo{}[never closed` hid a planted broken
    // reference — every construct after it was transported unread.
    let input = b"A sample: \\foo{}[never closed\n\\section{A}@id(sec:a)\nSee @ref(sec:a) and @ref(sec:ghost).\n";
    assert_eq!(
        constructs(input),
        [EntryToken::Id, EntryToken::Ref, EntryToken::Ref]
    );
    assert!(
        !scan(input)
            .iter()
            .any(|piece| matches!(piece, Piece::Quarantined(_)))
    );
    // A bracket that closes only past a blank line is prose too, for a
    // known signature as for an unknown command.
    for input in [
        &b"\\includegraphics[width=3cm\n\nSee @ref(after) ]"[..],
        b"\\foo[a\n\nb] @ref(after)",
        b"\\lstinline[never @ref(after)",
    ] {
        assert_eq!(
            constructs(input),
            [EntryToken::Ref],
            "{}",
            String::from_utf8_lossy(input)
        );
        assert!(
            !scan(input)
                .iter()
                .any(|piece| matches!(piece, Piece::Quarantined(_)))
        );
    }
    // A real optional argument may span a line, and still bounds its call.
    assert_eq!(
        constructs(b"\\caption[short\ntitle]{x @ref(inside)} @ref(after)"),
        [EntryToken::Ref, EntryToken::Ref]
    );
    assert_eq!(
        constructs(b"\\foo[a\nb @ref(hidden)] @ref(after)"),
        [EntryToken::Ref]
    );
}

#[test]
fn newif_defines_a_conditional_and_does_not_open_one() {
    // The E2 reproducer: `\newif\iffoo` in a preamble sent the rest of the
    // file to quarantine with no diagnostic. The definition claims its name;
    // a later use of the defined conditional is still a real region.
    let input = b"\\documentclass{article}\n\\newif\\iffoo\n\\begin{document}\n\\section{A}@id(sec:a)\nSee @ref(sec:a).\n\\end{document}\n";
    assert_eq!(constructs(input), [EntryToken::Id, EntryToken::Ref]);
    assert!(
        !scan(input)
            .iter()
            .any(|piece| matches!(piece, Piece::Quarantined(_)))
    );
    let used = b"\\newif\\iffoo \\iffoo @ref(hidden) \\fi @ref(after)";
    assert_eq!(constructs(used), [EntryToken::Ref]);
    let bare = b"\\newif @ref(after)";
    assert_eq!(constructs(bare), [EntryToken::Ref]);
}

#[test]
fn iff_in_prose_is_the_kernel_symbol_and_opens_nothing() {
    // `\iff` is ⟺, not a conditional; there is no `\fi` and never will
    // be. Before the fix this quarantined the rest of the file, which is
    // how the external corpus found it in real papers.
    let input = b"A \\iff B en prosa. @ref(after)";
    assert_eq!(constructs(input), [EntryToken::Ref]);
    assert!(
        !scan(input)
            .iter()
            .any(|piece| matches!(piece, Piece::Quarantined(_)))
    );
}

#[test]
fn iffalse_is_a_real_conditional_and_the_exception_must_not_reach_it() {
    // `\iffalse` begins with the same four bytes as `\iff`. A prefix
    // comparison excepts both; only the complete-name comparison excepts
    // one. The closed form scans as a conditional region; the unclosed
    // form quarantines.
    let closed = b"\\iffalse hidden @ref(hidden) \\fi @ref(after)";
    assert_eq!(constructs(closed), [EntryToken::Ref]);

    let unclosed = b"\\iffalse nunca cierra @ref(hidden)";
    assert_eq!(constructs(unclosed), []);
    assert!(
        scan(unclosed)
            .iter()
            .any(|piece| matches!(piece, Piece::Quarantined(_)))
    );
}

#[test]
fn iff_nested_inside_a_real_conditional_does_not_deepen_it() {
    // The nested-count site shares the rule. If `\iff` counted as a
    // nested conditional, this `\fi` would close depth two of three and
    // the region would swallow the file.
    let input = b"\\ifnum x \\iff y \\fi @ref(after)";
    assert_eq!(constructs(input), [EntryToken::Ref]);
    assert!(
        !scan(input)
            .iter()
            .any(|piece| matches!(piece, Piece::Quarantined(_)))
    );
}

#[test]
fn a_verb_delimiter_may_be_the_sigil_itself() {
    assert_eq!(constructs(b"\\verb|@ref(a)| @ref(b)"), [EntryToken::Ref]);
    assert_eq!(constructs(b"\\verb@@ref(a)@ @ref(b)"), [EntryToken::Ref]);
}

#[test]
fn verbatim_environments_hide_constructs() {
    for name in [
        "verbatim",
        "Verbatim",
        "verbatimtab",
        "listing",
        "lstlisting",
    ] {
        let input = format!("\\begin{{{name}}}\n@ref(a)\n\\end{{{name}}}\n@ref(b)");
        assert_eq!(
            constructs(input.as_bytes()),
            [EntryToken::Ref],
            "{name} did not hide its contents"
        );
    }
}

#[test]
fn the_internal_macro_region_hides_constructs() {
    assert_eq!(
        constructs(b"\\makeatletter @ref(a) \\makeatother @ref(b)"),
        [EntryToken::Ref]
    );
}

#[test]
fn a_raw_escape_hides_every_entry_token() {
    let input = b"latex {@id(a) @ref(b) @cite(c) @import(\"d\")} @ref(after)";
    assert_eq!(constructs(input), [EntryToken::Raw, EntryToken::Ref]);
}

#[test]
fn a_comment_inside_a_raw_escape_hides_its_closing_brace() {
    let input = b"latex {\n% a comment with } inside\n} @ref(after)";
    assert_eq!(constructs(input), [EntryToken::Raw, EntryToken::Ref]);
}

#[test]
fn the_bare_word_latex_is_not_an_entry_token() {
    assert_eq!(constructs(b"we use latex here"), []);
    assert_eq!(constructs(b"pdflatex {x}"), []);
    assert_eq!(constructs(b"\\latex {x}"), []);
}

#[test]
fn an_at_shape_that_is_not_a_keyword_is_text() {
    assert_eq!(constructs(b"name@example.org"), []);
    assert_eq!(constructs(b"@{}lcc@{}"), []);
    assert_eq!(constructs(b"@ref alone and @ref{a}"), []);
}

#[test]
fn an_unterminated_construct_is_reported_and_the_next_line_still_parses() {
    let pieces = scan(b"@ref(broken\n@ref(good)");
    let malformed: Vec<_> = pieces
        .iter()
        .filter(|p| matches!(p, Piece::Malformed { .. }))
        .collect();
    assert_eq!(malformed.len(), 1);
    assert_eq!(constructs(b"@ref(broken\n@ref(good)"), [EntryToken::Ref]);
}

#[test]
fn a_known_signature_excludes_exactly_its_arguments() {
    // \section is `s o m`. Since issue #83 its *mandatory* argument is
    // prose — a heading is a sentence — while the optional short title
    // stays data, the conservative default for the unclassified.
    assert_eq!(
        constructs(b"\\section[@ref(short)]{@ref(long)} @ref(after)"),
        [EntryToken::Ref, EntryToken::Ref]
    );
    assert_eq!(
        constructs(b"\\section*{@ref(a)} @ref(after)"),
        [EntryToken::Ref, EntryToken::Ref]
    );
    // A data-bearing command's arguments are still fully excluded.
    assert_eq!(
        constructs(b"\\includegraphics[width=@ref(x)]{@ref(path)} @id(after)"),
        [EntryToken::Id]
    );
}

#[test]
fn a_text_bearing_argument_is_prose_and_its_regions_compose() {
    // Phase 0a's gap 3, decided as issue #83: a caption is prose, so a
    // construct inside one is a construct — while every inner exclusion
    // still excludes. Each hidden case here carries exactly the bytes a
    // wrong implementation would convert.
    assert_eq!(
        constructs(b"\\caption{see @ref(fig:x)} @id(after)"),
        [EntryToken::Ref, EntryToken::Id]
    );
    assert_eq!(
        constructs(b"\\caption{a \\verb|@ref(v)| $@ref(m)$ % @ref(c)\nb @ref(real)}"),
        [EntryToken::Ref]
    );
    // Nested text-bearing commands scan through.
    assert_eq!(
        constructs(b"\\caption{\\textbf{@ref(deep)}}"),
        [EntryToken::Ref]
    );
    // A definition body is not prose, wherever a caption sits inside it.
    assert_eq!(constructs(b"\\newcommand{\\x}{\\caption{@ref(w)}}"), []);
    // `\item`'s prose is its optional argument.
    assert_eq!(
        constructs(b"\\item[see @ref(fig:x)] body"),
        [EntryToken::Ref]
    );
}

#[test]
fn a_command_takes_only_the_arguments_its_signature_declares() {
    // \emph is `m`. The second group is prose, not a second argument —
    // and since #83 the argument itself is prose too, so both refs are
    // recognised and the boundary is proven by their count staying two,
    // not three, when a third group follows nothing.
    assert_eq!(
        constructs(b"\\emph{@ref(arg)}{@ref(prose)}"),
        [EntryToken::Ref, EntryToken::Ref]
    );
}

#[test]
fn an_unknown_command_claims_adjacent_groups_up_to_the_bound() {
    let one = b"\\unknowncmd{@ref(a)} @ref(one)".to_vec();
    assert_eq!(constructs(&one), [EntryToken::Ref]);

    let sixteen: Vec<u8> = format!(
        "\\unknowncmd{} @ref(sixteen)",
        "{x}".repeat(MAX_UNKNOWN_COMMAND_GROUPS)
    )
    .into_bytes();
    assert_eq!(constructs(&sixteen), [EntryToken::Ref]);
}

#[test]
fn one_group_past_the_bound_stops_recognition_rather_than_guessing() {
    let past: Vec<u8> = format!(
        "\\unknowncmd{} @ref(never)",
        "{x}".repeat(MAX_UNKNOWN_COMMAND_GROUPS + 1)
    )
    .into_bytes();
    assert_eq!(constructs(&past), []);
    assert_eq!(reassemble(&past), past, "quarantine still transports");
}

#[test]
fn an_unbounded_argument_quarantines_rather_than_scanning_on() {
    let input = b"\\section{never closed @ref(a)";
    assert_eq!(constructs(input), []);
    assert_eq!(reassemble(input), input);
}

#[test]
fn label_has_an_optional_argument_which_is_easy_to_assume_it_lacks() {
    // The transcribed signature is `o m`, not `m`. A parser that assumed
    // `m` would read `[x]` as prose and recognise a construct inside it.
    assert_eq!(
        constructs(b"\\label[@ref(opt)]{@ref(name)} @ref(after)"),
        [EntryToken::Ref]
    );
}

#[test]
fn longest_keyword_wins() {
    // `@import(` must not be read as `@i` plus text, and `@id(` must not
    // shadow it.
    assert_eq!(constructs(b"@import(\"a.xtex\")"), [EntryToken::Import]);
    assert_eq!(constructs(b"@cite(k)"), [EntryToken::Cite]);
}
