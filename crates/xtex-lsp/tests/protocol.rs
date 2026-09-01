//! One test per message the server answers, feeding bytes and reading bytes.
//!
//! `docs/decisions/0005` requires this: an agent adding a method copies one of
//! these, and a framing mistake fails as a wrong byte rather than as an editor
//! that quietly does nothing.
//!
//! The server is driven as a subprocess over stdio, which is how an editor
//! drives it. Calling the handlers directly would test everything except the
//! part most likely to be wrong.

use std::io::Write;
use std::process::{Command, Stdio};

const DOCUMENT: &str = "\\section{Intro} @id(sec:intro)\n\
                        \\figure(fig:plot) { src = \"p.pdf\" caption = {C} }\n\
                        See @ref(fig:plot) and @ref(tab:none).\n";

/// Sends `bodies` as framed messages and returns the framed replies.
fn talk(bodies: &[String]) -> Vec<String> {
    let mut payload = Vec::new();
    for body in bodies {
        write!(payload, "Content-Length: {}\r\n\r\n{body}", body.len()).expect("write");
    }

    let mut child = Command::new(env!("CARGO_BIN_EXE_xtex-lsp"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("the server starts");
    child
        .stdin
        .take()
        .expect("stdin")
        .write_all(&payload)
        .expect("write to the server");
    let out = child.wait_with_output().expect("the server exits");

    let mut frames = Vec::new();
    let bytes = out.stdout;
    let mut at = 0usize;
    while at < bytes.len() {
        let head = bytes[at..]
            .windows(4)
            .position(|window| window == b"\r\n\r\n")
            .expect("a frame header")
            + at;
        let header = String::from_utf8_lossy(&bytes[at..head]).to_string();
        let length: usize = header
            .split("Content-Length:")
            .nth(1)
            .expect("a length")
            .trim()
            .parse()
            .expect("a number");
        let start = head + 4;
        frames.push(String::from_utf8_lossy(&bytes[start..start + length]).to_string());
        at = start + length;
    }
    frames
}

fn open(uri: &str) -> String {
    open_with(uri, DOCUMENT)
}

fn open_with(uri: &str, document: &str) -> String {
    let mut text = String::new();
    xtex_lsp_escape(document, &mut text);
    format!(
        r#"{{"jsonrpc":"2.0","method":"textDocument/didOpen","params":{{"textDocument":{{"uri":"{uri}","text":{text}}}}}}}"#
    )
}

fn xtex_lsp_escape(text: &str, out: &mut String) {
    out.push('"');
    for character in text.chars() {
        match character {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            other => out.push(other),
        }
    }
    out.push('"');
}

fn positional(id: u32, method: &str, line: u32, character: u32) -> String {
    positional_in(id, method, "file:///p.xtex", line, character)
}

fn positional_in(id: u32, method: &str, uri: &str, line: u32, character: u32) -> String {
    format!(
        r#"{{"jsonrpc":"2.0","id":{id},"method":"textDocument/{method}","params":{{"textDocument":{{"uri":"{uri}"}},"position":{{"line":{line},"character":{character}}}}}}}"#
    )
}

const EXIT: &str = r#"{"jsonrpc":"2.0","method":"exit","params":{}}"#;

#[test]
fn initialize_declares_what_the_server_answers() {
    let frames = talk(&[
        r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#.to_owned(),
        EXIT.to_owned(),
    ]);
    assert_eq!(frames.len(), 1, "{frames:?}");
    for capability in [
        "hoverProvider",
        "definitionProvider",
        "completionProvider",
        "textDocumentSync",
    ] {
        assert!(frames[0].contains(capability), "{}", frames[0]);
    }
}

#[test]
fn opening_a_document_publishes_the_same_diagnostics_the_cli_reports() {
    // The exit criterion of this phase, checked rather than argued: the code
    // and the message are the checker's, because the server calls the checker.
    let frames = talk(&[open("file:///p.xtex"), EXIT.to_owned()]);
    assert_eq!(frames.len(), 1, "{frames:?}");
    assert!(frames[0].contains("publishDiagnostics"), "{}", frames[0]);
    assert!(frames[0].contains("XT1003"), "{}", frames[0]);
    assert!(
        frames[0].contains("identifier `tab:none` is not declared"),
        "{}",
        frames[0]
    );
    // And nothing about the reference that resolves.
    assert!(!frames[0].contains("fig:plot"), "{}", frames[0]);
}

#[test]
fn hover_names_what_is_required_and_what_is_declared() {
    let frames = talk(&[
        open("file:///p.xtex"),
        positional(2, "hover", 2, 9),
        EXIT.to_owned(),
    ]);
    let hover = frames.last().expect("a reply");
    assert!(hover.contains("requires: figure"), "{hover}");
    assert!(hover.contains("declared as: figure"), "{hover}");
}

#[test]
fn completion_offers_only_what_the_prefix_demands() {
    let frames = talk(&[
        open("file:///p.xtex"),
        positional(3, "completion", 2, 9),
        EXIT.to_owned(),
    ]);
    let items = frames.last().expect("a reply");
    assert!(items.contains("fig:plot"), "{items}");
    // `sec:intro` is declared and is a section, so it is not offered to a
    // reference whose prefix demands a figure.
    assert!(!items.contains("sec:intro"), "{items}");
}

#[test]
fn definition_points_at_the_declaring_construct() {
    let frames = talk(&[
        open("file:///p.xtex"),
        positional(4, "definition", 2, 9),
        EXIT.to_owned(),
    ]);
    let reply = frames.last().expect("a reply");
    // The `\figure` block is the second line, zero-based line one.
    assert!(reply.contains(r#""line":1"#), "{reply}");
}

#[test]
fn a_citation_lands_on_its_bib_entry_in_its_own_file() {
    // The .bib must exist ON DISK beside the root: the loader reads it the
    // way an editor's project actually is, not through a fixture shortcut.
    let dir = std::env::temp_dir().join(format!("xtex-lsp-cite-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("tempdir");
    std::fs::write(
        dir.join("refs.bib"),
        "% classics\n@book{knuth1984, title={The TeXbook}}\n",
    )
    .expect("the bib is written");
    let uri = format!("file://{}/paper.tex", dir.display());
    let frames = talk(&[
        open_with(&uri, "See \\cite{knuth1984}.\n\\bibliography{refs}\n"),
        positional_in(6, "definition", &uri, 0, 12),
        EXIT.to_owned(),
    ]);
    let reply = frames.last().expect("a reply");
    assert!(
        reply.contains("refs.bib"),
        "the answer names the bib file: {reply}"
    );
    // The key's own token sits on the @book line — zero-based line one.
    assert!(reply.contains(r#""line":1"#), "{reply}");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_definition_reply_names_the_file_it_lands_in() {
    // The root's own constructs answer with the REQUEST's uri, not "" — an
    // empty uri sent every cross-file landing to the wrong buffer.
    let frames = talk(&[
        open("file:///p.xtex"),
        positional(7, "definition", 2, 9),
        EXIT.to_owned(),
    ]);
    let reply = frames.last().expect("a reply");
    assert!(reply.contains("file:///p.xtex"), "{reply}");
}

#[test]
fn a_position_in_prose_is_answered_with_null_rather_than_silence() {
    // An editor waiting for a reply that never comes looks like a hung server.
    let frames = talk(&[
        open("file:///p.xtex"),
        positional(5, "hover", 2, 1),
        EXIT.to_owned(),
    ]);
    assert!(
        frames.last().expect("a reply").contains(r#""result":null"#),
        "{frames:?}"
    );
}

#[test]
fn an_unhandled_method_is_not_answered_and_does_not_stop_the_server() {
    let frames = talk(&[
        r#"{"jsonrpc":"2.0","id":9,"method":"textDocument/formatting","params":{}}"#.to_owned(),
        r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#.to_owned(),
        EXIT.to_owned(),
    ]);
    assert_eq!(frames.len(), 1, "only initialize is answered: {frames:?}");
    assert!(frames[0].contains(r#""id":1"#), "{}", frames[0]);
}

#[test]
fn rename_edits_the_constructs_and_leaves_opaque_text_alone() {
    let body = r#"{"jsonrpc":"2.0","id":6,"method":"textDocument/rename","params":{"textDocument":{"uri":"file:///p.xtex"},"position":{"line":2,"character":9},"newName":"fig:arch"}}"#
        .to_owned();
    let frames = talk(&[open("file:///p.xtex"), body, EXIT.to_owned()]);
    let reply = frames.last().expect("a reply");
    assert!(reply.contains("fig:arch"), "{reply}");
    // Two edits: the declaration on line two and the reference on line three.
    assert_eq!(reply.matches(r#""newText""#).count(), 2, "{reply}");
}

#[test]
fn a_cleveref_reference_is_hovered_defined_and_renamed_like_ref() {
    // `@Cref(a, b)` names two identifiers. The cursor on the second one is
    // asked about; hover names the command as written and the key under the
    // cursor, definition lands on that key's declaration, and rename
    // rewrites the key inside the list.
    let document = "\\section{Intro} @id(sec:intro)\n\
                    \\figure(fig:plot) { src = \"p.pdf\" caption = {C} }\n\
                    See @Cref(sec:intro, fig:plot) and @autoref(fig:plot).\n";
    let uri = "file:///c.xtex";
    let frames = talk(&[
        open_with(uri, document),
        positional_in(2, "hover", uri, 2, 23),
        positional_in(3, "definition", uri, 2, 23),
        EXIT.to_owned(),
    ]);
    let published = &frames[0];
    assert!(
        !published.contains("XT1003"),
        "every name is declared: {published}"
    );
    let hover = &frames[1];
    assert!(hover.contains("@Cref(fig:plot)"), "{hover}");
    assert!(hover.contains("declared as: figure"), "{hover}");
    // The `\figure` block is the second line, zero-based line one.
    assert!(frames[2].contains(r#""line":1"#), "{}", frames[2]);

    let body = format!(
        r#"{{"jsonrpc":"2.0","id":4,"method":"textDocument/rename","params":{{"textDocument":{{"uri":"{uri}"}},"position":{{"line":2,"character":23}},"newName":"fig:arch"}}}}"#
    );
    let frames = talk(&[open_with(uri, document), body, EXIT.to_owned()]);
    let reply = frames.last().expect("a reply");
    // The block, the key inside the list, and the autoref.
    assert_eq!(reply.matches(r#""newText""#).count(), 3, "{reply}");
}

#[test]
fn a_construct_shaped_word_is_published_as_a_warning_not_an_error() {
    // XT2002 is an advisory: the CLI exits 0 on it, so the editor shows a
    // warning squiggle — severity 2 — never an error one.
    let frames = talk(&[
        open_with("file:///w.xtex", "As @eqref(eq:x) shows.\n"),
        EXIT.to_owned(),
    ]);
    let published = &frames[0];
    assert!(published.contains("XT2002"), "{published}");
    assert!(published.contains(r#""severity":2"#), "{published}");
    assert!(!published.contains(r#""severity":1"#), "{published}");
}

#[test]
fn prepare_rename_refuses_a_citation() {
    // Its key lives in a `.bib` this server does not own, and answering null
    // is how an editor is told not to open its rename box at all.
    let document = r#"{"jsonrpc":"2.0","method":"textDocument/didOpen","params":{"textDocument":{"uri":"file:///c.xtex","text":"@cite(knuth1984)"}}}"#;
    let ask = r#"{"jsonrpc":"2.0","id":7,"method":"textDocument/prepareRename","params":{"textDocument":{"uri":"file:///c.xtex"},"position":{"line":0,"character":8}}}"#;
    let frames = talk(&[document.to_owned(), ask.to_owned(), EXIT.to_owned()]);
    assert!(
        frames.last().expect("a reply").contains(r#""result":null"#),
        "{frames:?}"
    );
}

#[test]
fn prepare_rename_offers_the_construct_under_the_cursor() {
    let ask = r#"{"jsonrpc":"2.0","id":8,"method":"textDocument/prepareRename","params":{"textDocument":{"uri":"file:///p.xtex"},"position":{"line":2,"character":9}}}"#;
    let frames = talk(&[open("file:///p.xtex"), ask.to_owned(), EXIT.to_owned()]);
    let reply = frames.last().expect("a reply");
    assert!(reply.contains(r#""line":2"#), "{reply}");
}

#[test]
fn hover_resolves_a_name_declared_in_an_imported_file_on_disk() {
    // The buffer is the root and the project loads around it — the same
    // project-wide path the WebAssembly build uses, so the two hosts cannot
    // diverge. The imported file exists only on disk, so a file-local server
    // would answer "not declared in this document root".
    let dir = std::env::temp_dir().join(format!("xtex-lsp-cross-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("sections")).expect("a project dir");
    std::fs::write(
        dir.join("sections/model.xtex"),
        "\\section{M}@id(sec:model)\n",
    )
    .expect("the import");
    let root = dir.join("main.xtex");
    std::fs::write(&root, "").expect("the root exists for the uri");

    let text = "@import(\"sections/model.xtex\")\nVer @ref(sec:model).\n";
    let uri = format!("file://{}", root.display());
    let mut escaped = String::new();
    xtex_lsp_escape(text, &mut escaped);
    let open = format!(
        r#"{{"jsonrpc":"2.0","method":"textDocument/didOpen","params":{{"textDocument":{{"uri":"{uri}","text":{escaped}}}}}}}"#
    );
    let hover = format!(
        r#"{{"jsonrpc":"2.0","id":9,"method":"textDocument/hover","params":{{"textDocument":{{"uri":"{uri}"}},"position":{{"line":1,"character":9}}}}}}"#
    );
    let frames = talk(&[open, hover, EXIT.to_owned()]);
    let reply = frames.last().expect("a reply");
    assert!(
        reply.contains("declared as: section"),
        "the declaration lives only on disk; a file-local analysis cannot see it: {reply}"
    );
    std::fs::remove_dir_all(&dir).ok();
}
