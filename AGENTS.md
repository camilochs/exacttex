# AGENTS.md — ExactTeX

Universal instruction file for AI coding tools (Claude Code, Cursor, Copilot, Aider, OpenCode) and for any
human who wants the five-minute orientation. **Read this before making non-trivial changes.**

Positioning, design rules and the language allowed for claims live in [`PHILOSOPHY.md`](PHILOSOPHY.md). That
file is binding and this one does not restate it — two copies drift and then nobody knows which is current.

---

## 1 · Project intent

ExactTeX gives a writer information about the reliability of their document before they look at the PDF.

It is a one-directional superset of LaTeX: every valid `.tex` is valid input, and a `.xtex` need not compile
under plain TeX. LaTeX stays the backend and the artifact of record.

The two things that justify the project — and the two that must never be traded away — are stated in
`PHILOSOPHY.md` §3: **errors restated in the author's own words**, and **a change model inside the file**.
Faithful byte transport is what makes both reachable from documents that already exist.

---

## 2 · Glossary

- **Entity** — a figure, table, equation, section or citation that the author declared, and therefore that
  the tooling can name. Declared with `@id(x)` on any LaTeX construct, or by a typed block.
- **Light annotation** — `@id(x)` hung off existing LaTeX. Buys checked references and safe rename.
- **Full annotation** — `\figure(x) { … }` with typed fields. Buys visual checks and package synthesis.
- **Opaque region** — source ExactTeX does not model. Transported byte for byte, never rejected. Its type is
  `?O`, the unknown *open* datatype: any package may define new constructors.
- **Coverage** — the fraction of a document that is checked rather than opaque. The analogue of
  `noImplicitAny`. Reported by `xtex check`.
- **Transport** vs **convert** — transport returns the input bytes unchanged; convert produces a different
  artifact. ExactTeX transports. Converters are a one-way trip.
- **Blame** — which side of a boundary a failure belongs to: author LaTeX, a ExactTeX construct, or emitted
  output. A diagnostic without blame is unfinished.

---

## 3 · Repo layout

```
src/
  main.rs  cli.rs  source.rs  lexer.rs
  parser/ { mod.rs, native.rs, latex.rs, raw.rs }
  ast.rs          Span on every node; Opaque node holding raw source
  quarantine.rs   parse-confidence downgrade
  resolve.rs      symbol table, cross-file merge
  types.rs  check.rs
  review.rs       change constructs + sidecar
  emit.rs  sourcemap.rs  texlog.rs
  diagnostics.rs  one model, two renderers (human / JSON)
  bibliography.rs project.rs
  lsp/
tests/  corpus/
```

Directories appear as the tasks that create them land; this is the target shape, not a claim about what
exists today.

---

## 4 · Invariants — never break these

- **Untouched LaTeX comes out byte-identical.** `emit(parse(u)) == u` for input containing no ExactTeX
  constructs. A transporter that sometimes reformats is not a slower transporter, it is a false one, and the
  file it corrupts is an already-accepted paper.
- **Annotating never changes the PDF.** Adding a valid annotation must not alter a rendered pixel, and must
  not turn a passing build into a failing one. Test by fuzzing annotations and comparing rasters.
- **Erasure, never injection.** No assertions, wrapper environments or support packages are written into the
  output. Injection breaks the invariant above and collides with packages and catcodes.
- **Opaque bytes are never normalised.** Spans point into immutable source buffers and emission copies the
  original slice. Any emitter "improvement" over an opaque node — reindenting, collapsing whitespace,
  reordering arguments — breaks transport silently.
- **A hard error only ever comes from an explicit ExactTeX construct.** `?O` is consistent with every type, so
  an unannotated `\ref{x}` cannot fail. A renamed `.tex` checks clean by construction. Everything else is
  `advisory`, behind `--strict-tex`, and never touches the exit code.
- **Unknown LaTeX is never a fatal error.** The parser downgrades confidence and preserves. Fatal is reserved
  for I/O failure, invalid annotation encoding, resource limits, and broken internal invariants.
- **A diagnostic names its blame side.** Author LaTeX, ExactTeX construct, or emitted output. With no map
  segment to support it, the answer is `unresolved` — never a guess.
- **An entry token must not be able to appear in ordinary LaTeX prose.** Otherwise renaming a `.tex` silently
  changes its meaning.
- **Numbers in docs have a command behind them.** If a README or a PR claims a coverage figure, a runtime or
  an error rate, a documented command reproduces it.
- **Dependencies are permissively licensed.** MIT, Apache-2.0, BSD, ISC. SPDX identifier read from package
  metadata, not recalled. GPL, AGPL and SSPL are never proposed — ExactTeX is MIT, which is why texlab's
  parser cannot be reused.

---

## 5 · Anti-patterns — mistakes already made here

Every one of these was proposed during design and looked reasonable at the time.

- **Justifying a syntax decision by counting characters.** The metric is how much the tooling learns, not how
  little you type. This was proposed immediately after the opposite was established.
- **Claiming the typed syntax as the contribution.** MyST and sTeX already have it. See `PHILOSOPHY.md` §3
  and §7 for what may be claimed.
- **Comparing a competitor by overlapping features.** The question is what each tool demands of the user.
  Typst asks an author to abandon their corpus; ExactTeX asks them to keep it. Same features, different
  strategies, different people.
- **Scanning inside an opaque region for `\label` / `\ref` / `\cite` and treating hits as checkable.** Such a
  scan matches inside a `\newcommand` body, inside verbatim text, and inside an inactive `\if` branch.
  Advisory only, never a hard error.
- **A per-file pointer to the project root.** A project can have several roots — one real paper had five.
  The root is found by walking up to the nearest `xtex.toml`.
- **A structural check that ignores the constructs that legitimately break it.** A column count that does not
  sum `\multicolumn` widths reports false positives on ordinary tables.
- **Treating a prediction as a finding.** Nothing that is not built is described as working. An anticipated
  risk is labelled a prediction and carries the cheap check that would settle it.
- **Shipping generated code unread.** Over-production is the tell: defensive branches for impossible states,
  helpers used once, doc comments restating the signature. Delete them.
- **Reviewing the diff instead of the artifact.** A specification produced elsewhere was checked only where
  it had changed — the corrections matched their evidence, so it was declared done. Five features it admits
  were unspecified: where `@cite` keys come from, the scope of an identifier, how to escape an entry token,
  where `@id` attaches on an equation, and what `@id` emits. Reviewing a change is not reviewing a document.
  Before accepting one, walk every feature it claims to admit and check it is specified.
- **Destructive shell commands against a path you did not inspect.** Move to a dated quarantine directory
  instead, and check the target first — macOS filesystems are case-insensitive.

---

## 6 · Where to look first

| If you're working on… | Start at |
|---|---|
| What ExactTeX may and may not claim | `PHILOSOPHY.md` §3, §7 |
| A syntax decision | `PHILOSOPHY.md` §4, then the grammar |
| Whether a construct may fail hard | `PHILOSOPHY.md` §6, then `check.rs` |
| Byte transport breaking | `ast.rs` (the Opaque node), then `emit.rs` |
| A LaTeX construct that confuses the parser | `parser/raw.rs`, `quarantine.rs` |
| A TeX error pointing at the wrong place | `sourcemap.rs`, `texlog.rs` |

---

## 7 · Where this project belongs

ExactTeX is an **AF Labs** project. It is not affiliated with any other organisation, and no external
engineering handbook governs it: the engineering rules in §8 live here and here only.

What AF Labs does bind is **how claims are made and tested**, and those protocols apply inside this repo:

- **A claim of completion is not evidence.** "Done", "fixed", "working", "the tests pass" — asserted by the
  same chain that wrote the code — is self-report. Every completion claim carries one falsifier and one
  observation produced by something other than the chain that made it.
- **A measurement carries the fingerprint of its input.** A result file that does not say which question it
  answers, under which code, against which reference, can be picked up downstream and treated as current. A
  pre-flight checks fidelity, not presence: counting files is not verification.
- **An external fact is transcribed from a source opened at that moment.** Licences, API behaviour,
  citations, competitor capabilities. Composed-from-memory is the failure mode, and a confident wrong
  reference is worse than a missing one.
- **A verdict that gates a decision is replicated, or labelled `single-draw`.** One run of one model is one
  draw.
- **No anthropomorphism.** See `PHILOSOPHY.md` §8. Binding in code comments, diagnostics, docs and commits.
- **Diagnostics and docs are written in plain language.** A technical term appears only when necessary and is
  explained in place. This is a product rule here, not only a style rule: §3.1 of `PHILOSOPHY.md` is the
  whole point of the project.

---

## 8 · Workflow

- **Issue → branch → PR → review → squash-merge.** Never push to `main`. There is no `dev` branch.
- **Every issue names the documentation its fix must update.** `None` is a valid answer, never a default.
- **PR body includes `Fixes #N`.** CI rejects PRs without it.
- **One issue per PR.** No drive-by refactor, no drive-by reformat.
- **Delete the branch when the PR merges**, and never commit to a merged branch — follow-up work is a new
  issue and a new branch.
- **Never credit a tool.** No `Co-Authored-By` naming an agent, no "Generated with" line, no robot emoji, in
  commits, PRs, issues, comments, or any document in this repo. The human contributor is the sole author and
  is accountable for every line.
- **Style is settled by the tools, never in review.** `cargo fmt`, `cargo clippy`. Doc comments state the
  contract; inline comments say *why*, never *what*. `TODO(#N)` or nothing.

### The single reviewer

This repo has one maintainer, who reviews every PR. That removes the second pair of eyes, so it is
manufactured instead:

- **Run the diff past a decorrelated model arm before requesting review.** An agent writing and one human
  reviewing is a two-link chain whose second link is the only filter. A second model lineage reading the diff
  is not a second opinion; it is a substrate whose errors do not correlate with the one that wrote the code.
  It never authors the fix — it locates defects, and the maintainer decides what to adopt.
- **The PR carries the evidence; the reviewer does not reconstruct it.** A PR whose test plan has no real
  command output is returned unread, not reviewed partially.
- **The fix lives where the defect is.** A workaround one layer away from the cause is a new defect wearing a
  fix's clothes.
