# Architecture

One diagram, and the four commitments it draws.

![ExactTeX architecture: a project enters xtex-core, whose scanner, document model and symbol table feed check, emit, editor, review and blame; three surfaces — CLI, LSP and WebAssembly — call the same core and answer identically; unannotated bytes pass through byte-identical; below, behind its own door, xtex claims feeds xtex-verify, which writes the dated .xtexverified record that check --verified replays offline.](assets/architecture.svg)

## The four commitments in the picture

**Unannotated bytes pass through byte-identical.** The dashed line across
the top is the transport guarantee: what you never annotated is never
touched. It is drawn outside the pipeline on purpose — opaque bytes are
carried, not processed, and every stage below is forbidden to normalise
them.

**One rule decides where the document carries text.** The scanner owns
that decision, and everything downstream — recognition, emission, review,
rename — reads it from the scanner rather than deciding again. When two
paths decided separately, a revision inside `\author{…}` was resolvable
on screen and printed into the PDF at the same time; the single rule is
what makes that disagreement impossible.

**Every surface answers identically.** `xtex-cli`, `xtex-lsp` and
`xtex-wasm` are thin: each one decodes its transport and calls the same
`xtex-core` functions. The parity suite in CI holds the answers
byte-identical, so "works in the terminal, differs in the browser" is a
failing test rather than a bug report.

**The network lives behind its own door.** The bottom band is external
verification: the compiler lists the document's claims, a separate
verifier asks the live sources about them, and what crosses back is a
dated record the deterministic check replays offline. A compile never
waits on the network, and a timeout is never mistaken for nonexistence.
The full walk: [verification.md](verification.md).

## Where each box lives

| box | code |
|---|---|
| scanner | `crates/xtex-core/src/scanner/` |
| document model | `crates/xtex-core/src/document.rs` |
| symbol table · signatures | `crates/xtex-core/src/symbols.rs`, `signatures.rs` |
| check | `crates/xtex-core/src/check.rs` |
| emit · sourcemap | `crates/xtex-core/src/lib.rs`, `sourcemap.rs` |
| editor queries | `crates/xtex-core/src/editor.rs` |
| review · rename | `crates/xtex-core/src/review.rs`, `rename.rs` |
| blame · texlog | `crates/xtex-core/src/blame.rs`, `texlog.rs` |
| surfaces | `crates/xtex-cli/`, `crates/xtex-lsp/`, `crates/xtex-wasm/` |
| external verification | `crates/xtex-verify/` (excluded from the workspace), `crates/xtex-core/src/claims.rs`, `verification.rs` |

`xtex-core` has zero dependencies, which is not an aesthetic: it is what
lets the identical library compile to the CLI, the language server and a
WebAssembly module with no glue in between.
