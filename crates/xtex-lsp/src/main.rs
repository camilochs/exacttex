//! The ExactTeX language server.
//!
//! # How to add a message
//!
//! One table maps a method to a handler, in [`handle`]. Add a case there and a
//! function beside it. There is no trait to implement and no registration
//! order that matters, and that is on purpose: `docs/decisions/0005` records
//! that this server is written by hand and has to stay something an agent can
//! change safely.
//!
//! Handlers hold no logic. Every question an editor asks is answered by
//! `xtex_core::editor`, which is testable without a protocol — so "the server
//! and the CLI report the same thing" is true by construction rather than by
//! comparison.
//!
//! A message not in the table is not answered, and the editor's own fallback
//! is the correct behaviour.

mod json;
mod rpc;

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::io::{BufReader, Write};

use json::{Value, write_text};
use xtex_core::bibliography::{Bibliography, Unavailable};
use xtex_core::check::check;
use xtex_core::document::Document;
use xtex_core::editor::{Position, completions, construct_at, definition, hover, offset_at};
use xtex_core::parse;
use xtex_core::rename::plan;
use xtex_core::scanner::EntryToken;
use xtex_core::source::Sources;
use xtex_core::symbols::SymbolTable;

/// Documents the editor has opened, by URI.
type Open = BTreeMap<String, String>;

fn main() -> std::io::Result<()> {
    let stdin = std::io::stdin();
    let mut input = BufReader::new(stdin.lock());
    let mut output = std::io::stdout();
    let mut open = Open::new();

    while let Some(message) = rpc::read(&mut input)? {
        if message.method == "shutdown" {
            if let Some(id) = message.id {
                rpc::write(&mut output, &rpc::reply(id, "null"))?;
            }
            continue;
        }
        if message.method == "exit" {
            return Ok(());
        }
        for frame in handle(&message, &mut open) {
            rpc::write(&mut output, &frame)?;
        }
        output.flush()?;
    }
    Ok(())
}

/// The table. One line per message this server answers.
fn handle(message: &rpc::Message, open: &mut Open) -> Vec<String> {
    match message.method.as_str() {
        "initialize" => message.id.map(initialize).into_iter().collect(),
        "textDocument/didOpen" | "textDocument/didChange" => {
            did_change(&message.params, open).into_iter().collect()
        }
        "textDocument/hover" => reply_with(message, open, on_hover),
        "textDocument/completion" => reply_with(message, open, on_completion),
        "textDocument/definition" => reply_with(message, open, on_definition),
        "textDocument/prepareRename" => reply_with(message, open, on_prepare_rename),
        "textDocument/rename" => on_rename(message, open),
        _ => Vec::new(),
    }
}

/// Answers a positional request, or replies `null` when there is nothing there.
fn reply_with(
    message: &rpc::Message,
    open: &Open,
    answer: impl Fn(&str, Position) -> Option<String>,
) -> Vec<String> {
    let Some(id) = message.id else {
        return Vec::new();
    };
    let Some((uri, position)) = locate(&message.params) else {
        return vec![rpc::reply(id, "null")];
    };
    let Some(text) = open.get(&uri) else {
        return vec![rpc::reply(id, "null")];
    };
    let body = answer(text, position).unwrap_or_else(|| "null".to_owned());
    vec![rpc::reply(id, &body)]
}

fn initialize(id: i64) -> String {
    rpc::reply(
        id,
        r#"{"capabilities":{"textDocumentSync":1,"hoverProvider":true,"definitionProvider":true,"completionProvider":{"triggerCharacters":["(",":"]},"renameProvider":{"prepareProvider":true}},"serverInfo":{"name":"xtex-lsp"}}"#,
    )
}

/// Records the document and publishes its diagnostics.
///
/// The editor is told on every change, because a diagnostic that arrives only
/// on save is a diagnostic the author has already worked past.
fn did_change(params: &Value, open: &mut Open) -> Option<String> {
    let document = params.get("textDocument")?;
    let uri = document.get("uri")?.text()?.to_owned();
    let text = document
        .get("text")
        .and_then(Value::text)
        .map(str::to_owned)
        .or_else(|| {
            // A full-sync change carries one item holding the whole document.
            match params.get("contentChanges")? {
                Value::List(items) => items.last()?.get("text")?.text().map(str::to_owned),
                _ => None,
            }
        })?;

    let diagnostics = diagnose(&uri, &text);
    open.insert(uri.clone(), text);
    Some(diagnostics)
}

/// The same diagnostics `xtex check` reports, rendered for an editor.
fn diagnose(uri: &str, text: &str) -> String {
    let mut sources = Sources::new();
    let id = sources.add(uri, text.as_bytes().to_vec());
    let document = parse(&sources, id);
    let mut table = SymbolTable::new();
    table.merge(&sources, &document);
    // No bibliography is read here: the server has no project root, and
    // `Unavailable` is exactly the state that keeps every citation silent
    // rather than reported as missing.
    let bibliography = Bibliography::Unavailable(Unavailable::NoneDeclared);

    let mut items = String::new();
    for diagnostic in check(&table, &bibliography) {
        let (line, column) = line_column(text.as_bytes(), diagnostic.span.start());
        let (end_line, end_column) = line_column(text.as_bytes(), diagnostic.span.end());
        if !items.is_empty() {
            items.push(',');
        }
        let _ = write!(
            items,
            r#"{{"range":{{"start":{{"line":{},"character":{}}},"end":{{"line":{},"character":{}}}}},"severity":1,"code":"{}","source":"xtex","message":"#,
            line, column, end_line, end_column, diagnostic.code
        );
        write_text(&diagnostic.message, &mut items);
        items.push('}');
    }

    let mut params = String::from(r#"{"uri":"#);
    write_text(uri, &mut params);
    let _ = write!(params, r#","diagnostics":[{items}]}}"#);
    rpc::notify("textDocument/publishDiagnostics", &params)
}

fn on_hover(text: &str, position: Position) -> Option<String> {
    let (sources, document, table) = analyse(text);
    let offset = offset_at(text.as_bytes(), position)?;
    let found = hover(&sources, &document, &table, offset)?;
    let mut body = String::from(r#"{"contents":{"kind":"plaintext","value":"#);
    write_text(&found.text, &mut body);
    body.push_str("}}");
    Some(body)
}

fn on_completion(text: &str, position: Position) -> Option<String> {
    let (sources, document, table) = analyse(text);
    let offset = offset_at(text.as_bytes(), position)?;
    let items = completions(&sources, &document, &table, offset);
    let mut body = String::from("[");
    for (index, item) in items.iter().enumerate() {
        if index > 0 {
            body.push(',');
        }
        body.push_str(r#"{"label":"#);
        write_text(&item.label, &mut body);
        body.push_str(r#","detail":"#);
        write_text(
            item.detail.as_deref().unwrap_or(item.class.name()),
            &mut body,
        );
        body.push('}');
    }
    body.push(']');
    Some(body)
}

fn on_definition(text: &str, position: Position) -> Option<String> {
    let (sources, document, table) = analyse(text);
    let offset = offset_at(text.as_bytes(), position)?;
    let span = definition(&sources, &document, &table, offset)?;
    let (line, column) = line_column(text.as_bytes(), span.start());
    let (end_line, end_column) = line_column(text.as_bytes(), span.end());
    Some(format!(
        r#"{{"uri":"","range":{{"start":{{"line":{line},"character":{column}}},"end":{{"line":{end_line},"character":{end_column}}}}}}}"#
    ))
}

/// Parses the document and builds its table, which is what every positional
/// handler needs and none of them should assemble itself.
fn analyse(text: &str) -> (Sources, Document, SymbolTable) {
    let mut sources = Sources::new();
    let id = sources.add("document.xtex", text.as_bytes().to_vec());
    let document = parse(&sources, id);
    let mut table = SymbolTable::new();
    table.merge(&sources, &document);
    (sources, document, table)
}

/// The URI and position a positional request names.
fn locate(params: &Value) -> Option<(String, Position)> {
    let uri = params.get("textDocument")?.get("uri")?.text()?.to_owned();
    let position = params.get("position")?;
    // LSP counts from zero and this compiler counts from one.
    let line = u32::try_from(position.get("line")?.integer()?).ok()? + 1;
    let column = u32::try_from(position.get("character")?.integer()?).ok()? + 1;
    Some((uri, Position { line, column }))
}

/// Zero-based line and column of a byte offset, which is what LSP wants.
// Clippy suggests the `bytecount` crate here. `docs/decisions/0005` is why we
// do not take it: this server carries no dependencies, and the counting is
// over one line's worth of bytes per diagnostic.
#[allow(clippy::naive_bytecount)]
fn line_column(bytes: &[u8], offset: usize) -> (usize, usize) {
    let before = &bytes[..offset.min(bytes.len())];
    let line = before.iter().filter(|byte| **byte == b'\n').count();
    let start = before
        .iter()
        .rposition(|byte| *byte == b'\n')
        .map_or(0, |at| at + 1);
    (line, offset.saturating_sub(start))
}

/// The range an editor should offer to edit, or `null` where renaming is not
/// possible.
///
/// Answering `null` is how an editor is told not to open its rename box at
/// all, which is better than opening one whose result would be refused.
fn on_prepare_rename(text: &str, position: Position) -> Option<String> {
    let (sources, document, _) = analyse(text);
    let offset = offset_at(text.as_bytes(), position)?;
    let located = construct_at(&sources, &document, offset)?;
    if located.kind == EntryToken::Cite {
        // Its key lives in a `.bib` this server does not own.
        return None;
    }
    let (line, column) = line_column(text.as_bytes(), located.span.start());
    let (end_line, end_column) = line_column(text.as_bytes(), located.span.end());
    Some(format!(
        r#"{{"start":{{"line":{line},"character":{column}}},"end":{{"line":{end_line},"character":{end_column}}}}}"#
    ))
}

/// A workspace edit renaming every structurally resolved occurrence.
///
/// Occurrences in opaque text are deliberately absent from the edit. The
/// server has no channel to explain that in a rename reply, so `xtex rename`
/// is where an author is told; `docs/lsp.md` says so.
fn on_rename(message: &rpc::Message, open: &Open) -> Vec<String> {
    let Some(id) = message.id else {
        return Vec::new();
    };
    let Some((uri, position)) = locate(&message.params) else {
        return vec![rpc::reply(id, "null")];
    };
    let (Some(text), Some(new_name)) = (
        open.get(&uri),
        message.params.get("newName").and_then(Value::text),
    ) else {
        return vec![rpc::reply(id, "null")];
    };

    let (sources, document, _) = analyse(text);
    let Some(offset) = offset_at(text.as_bytes(), position) else {
        return vec![rpc::reply(id, "null")];
    };
    let Some(located) = construct_at(&sources, &document, offset) else {
        return vec![rpc::reply(id, "null")];
    };
    let plan = plan(
        &sources,
        std::slice::from_ref(&document),
        &located.name,
        new_name,
    );

    let mut edits = String::new();
    for edit in &plan.edits {
        let (line, column) = line_column(text.as_bytes(), edit.span.start());
        let (end_line, end_column) = line_column(text.as_bytes(), edit.span.end());
        if !edits.is_empty() {
            edits.push(',');
        }
        let _ = write!(
            edits,
            r#"{{"range":{{"start":{{"line":{line},"character":{column}}},"end":{{"line":{end_line},"character":{end_column}}}}},"newText":"#
        );
        write_text(new_name, &mut edits);
        edits.push('}');
    }

    let mut body = String::from(r#"{"changes":{"#);
    write_text(&uri, &mut body);
    let _ = write!(body, ":[{edits}]}}}}");
    vec![rpc::reply(id, &body)]
}
