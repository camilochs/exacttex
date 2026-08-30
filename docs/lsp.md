# The language server

`xtex-lsp` speaks LSP over stdin and stdout. It answers nine messages and no others.

---

## What it answers

| Message | What it does |
|---|---|
| `initialize` | Declares the capabilities below. |
| `textDocument/didOpen` | Records the document, publishes its diagnostics. |
| `textDocument/didChange` | The same, on every keystroke the editor sends. |
| `textDocument/hover` | What the construct under the cursor is, and whether it resolves. |
| `textDocument/completion` | Identifiers that may go where the cursor is. |
| `textDocument/definition` | Where the name under the cursor is declared. |
| `textDocument/prepareRename` | The range the editor should offer, or `null` where renaming is not possible. |
| `textDocument/rename` | Edits for every structurally resolved occurrence. |
| `shutdown` / `exit` | Replies `null`, then leaves. |

**Anything else is not answered, and that is correct behaviour.** The editor's own fallback — its word-based
completion, its "no definition found" — is better than a wrong answer from a server that does not model the
question. A message that is not in this table has no code behind it.

Capabilities declared: `textDocumentSync: 1` (full text on every change), `hoverProvider`,
`definitionProvider`, `completionProvider` triggered on `(` and `:`, and `renameProvider` with
`prepareProvider`.

---

## Diagnostics are the checker's, not the server's

The server calls `check`. It has no diagnostics of its own and never filters or rewords the checker's:
`xtex check` and the editor show the same code, the same message and the same span for the same input,
because one implementation answers both.

That is the exit criterion of this phase, and `opening_a_document_publishes_the_same_diagnostics_the_cli_reports`
is where it is checked.

One difference is deliberate: **the server reads no bibliography.** It is handed a document, not a project
root, so the bibliography is `Unavailable` — which is exactly the state that keeps every `@cite` silent
rather than reported as missing. An editor that flagged every citation in a file it could not resolve would train the author to ignore the warning.

---

## Hover

```
@ref(fig:plot)
requires: figure
declared as: figure
```

Every line is a fact the compiler already holds. `requires:` is the class the prefix demands
([`decisions/0003`](decisions/0003-the-prefix-is-the-demand.md)); `declared as:` is what the target actually
is. When they disagree you are looking at `XT1004` before the compiler has run.

An unresolved reference says `not declared in this document root` rather than showing an empty popup, which
is what an author sees for most of the time they are typing a name.

---

## Completion

Inside `@ref(`, the prefix already typed **is** the filter. `@ref(tab:` is offered tables and nothing else,
because offering a figure there is offering an error.

This is the type system paying for itself in the editor rather than only in the exit code. An identifier
whose class is `?O` is always offered, because `?O` is consistent with everything and the compiler has no
grounds to exclude it.

Outside a construct, nothing is offered. Outside a construct, suggesting every identifier in the document turns completion into noise.

---

## Rename, and the thing it will not do

Renaming changes every occurrence the compiler **structurally resolved** — the declaration, every `@ref` to
it in the document root, and a `@note`'s `on =` field, which is a reference and would otherwise be orphaned.

It changes nothing else, and the gap is the whole design:

```latex
@id(fig:plot)                              renamed
@ref(fig:plot)                             renamed
\label{fig:plot}                           left alone
\verb|fig:plot|                            left alone
We call it fig:plot in the text.           left alone
@cite(fig:plot)                            left alone — a bibliography key
```

`@id(fig:plot)` emits `\label{fig:plot}`, so an author may also have written a plain `\ref{fig:plot}`. That
is transported LaTeX: unchecked, and by `AGENTS.md` §4 never rewritten. Renaming the construct and silently
leaving that `\ref` behind would break a working document; rewriting it would break the invariant. So
neither happens — and the occurrences left alone are **reported**:

```
$ xtex rename main.xtex fig:model fig:architecture
main.xtex: 2 renamed
sections/part.xtex: 2 renamed
  left alone: main.xtex:4:28 — transported LaTeX is never rewritten
  left alone: main.xtex:4:58 — transported LaTeX is never rewritten
  left alone: main.xtex:5:12 — transported LaTeX is never rewritten
```

The editor gets the edits; it has no channel for that list, so `xtex rename` is where an author is told.
An editor renaming a document with plain `\ref` in it should expect to hear about it from the CLI.

**The scope is the document root**, not the open file — an identifier is shared by the root file and
everything it imports, so renaming one file at a time would leave the rest pointing at a name that no longer
exists.

A citation cannot be renamed at all. `prepareRename` answers `null` on one, which is how an editor is told
not to open its rename box; its key lives in a `.bib` this does not own.

---

## Running it

```sh
cargo build --release -p xtex-lsp
```

The binary is `xtex-lsp` and it takes no arguments. Point your editor at it for the `xtex` language and
`.xtex` files; the setup is whatever your editor calls "a custom language server command", and nothing here
is editor-specific.

There is no configuration. A project's `xtex.toml` is read by the compiler, not by the server.

---

## Why it is written by hand

`tower-lsp` and `lsp-types` were both available and both permissively licensed. Neither is used, and
[`decisions/0005`](decisions/0005-the-language-server-is-written-by-hand.md) records why and — more usefully
— the four rules that keep a hand-written server maintainable.

The one that matters for anyone changing it: **the protocol layer holds no logic.** It frames bytes, reads a
method name, calls a function, writes bytes back. Every question is answered by `xtex_core::editor`, in
functions that take bytes and offsets and return data, testable with no server running.

To add a message: add a case to the table in `handle`, add a function beside it, and add a test to
`crates/xtex-lsp/tests/protocol.rs` that feeds bytes and asserts bytes.
