# Roadmap

## Purpose

NextTeX gives a writer information about the reliability of a document before they inspect the PDF.

It is a one-directional superset of LaTeX. Every valid `.tex` file is valid NextTeX input, while a `.ntex`
file may contain constructs that plain TeX cannot process. LaTeX remains the typesetting backend and the
artifact submitted to journals.

The project combines:

- diagnostics stated using author-declared entity names;
- revisions stored as structured data in the document format;
- faithful transport of existing LaTeX;
- checked references, citations, figures, tables, paths, lengths, and revision identifiers;
- editor operations over one project-wide document model.

Typed document languages, semantic LaTeX annotations, and LaTeX language servers already exist. The
admissible claim is that this combination is not assembled. See [`PHILOSOPHY.md`](PHILOSOPHY.md) §3 and §7.

## Correctness properties

### A. Transport

For any input byte sequence `u` containing no NextTeX constructs:

```text
emit(parse(u)).tex == u
check(parse(u)) produces no hard errors
```

The first comparison is byte equality, not textual or Unicode equivalence. Line endings, encodings,
comments, whitespace, and opaque macro bodies must remain unchanged.

A renamed `.tex` checking clean means that NextTeX emits no hard diagnostics of its own. It does not mean
that the input compiles successfully under TeX.

A transport test fails if

```bash
cmp input.tex build/input.tex
```

reports any difference.

### B. Typesetting equivalence

For every document `d` that passes checking:

```text
render(tex(emit(d).tex)) == render(tex(emit(erase(d)).tex))
tex(emit(d).tex).status == tex(emit(erase(d)).tex).status
```

`render` uses a pinned TeX engine, removes declared volatile metadata, rasterizes pages under fixed
settings, and compares page structure and pixels under a declared tolerance.

The property fails if adding a valid annotation changes a normalized rendered pixel, or changes a successful
TeX invocation into a failed one.

NextTeX uses erasure, not injected assertions, wrapper environments, or support packages. Such injection
could collide with packages or catcodes and would violate this property.

## Project and output structure

A project is located by walking upward to the nearest `nextex.toml`. A project may declare several document
roots.

A normal build emits **one `.tex` file for every `.ntex` file** and mirrors the source layout under
`build/`:

```text
paper/main.ntex                 ->  build/paper/main.tex
paper/sections/model.ntex       ->  build/paper/sections/model.tex
paper/appendices/proofs.ntex    ->  build/paper/appendices/proofs.tex
```

The normal emitter does not flatten a project into one file. Flattening would inline `\input` or
`\include`, thereby transforming LaTeX the author did not touch and breaking transport byte identity.

`--flatten` is an explicit opt-in for single-file journal submission. Its diagnostics and documentation must
state that flattened output is a conversion artifact and does not satisfy transport byte equality.

Paths in `@import`, `src`, and similar fields resolve relative to the file containing the field. Imported
symbol tables merge at project scope, while emission preserves file boundaries.

## Compiler architecture

The implementation is in Rust. Native and WebAssembly builds use the same core crates.

All source access, project discovery, output storage, bibliography access, and TeX invocation sit behind I/O
traits. The compiler core must not contain assumptions about host paths, process spawning, current
directories, or direct filesystem access.

WebAssembly is a first-class output alongside the native binary. Rust makes the additional target a small
architectural extension, provided that I/O is isolated from the core. It is the route to a browser surface
without a compile server. TeX already runs in browsers through WASM: SwiftLaTeX does so, and TeXlyre-BusyTeX
supports TeX Live 2026 with pdfLaTeX, XeLaTeX, and LuaLaTeX.

### Two front doors

Both inputs converge on the same `Document` model:

1. **Native front door** — parses explicit NextTeX syntax: `@id`, `@ref`, `@cite`, `@import`, typed blocks,
   revision constructs, and delimited raw-LaTeX escapes.
2. **LaTeX front door** — shallow parsing: recognizes safe boundaries, records selected declarations, and
   represents everything else as opaque source spans.

Neither front door owns resolution, checking, emission, diagnostics, or editor behavior.

### Five compiler stages

1. **Parse and preserve**
   - Load immutable byte buffers through the I/O trait.
   - Select the native or shallow-LaTeX front door.
   - Produce a shared `Document` with spans on every node.
   - Preserve unmodelled bytes in `Opaque` nodes.
   - Record `ParseConfidence::{Structured, OpaqueBalanced, OpaqueToEof}`.

2. **Resolve**
   - Discover the nearest `nextex.toml`.
   - Resolve literal imports and file-relative paths.
   - Merge declarations into a project-wide symbol table.
   - Load successfully parsed bibliographies.
   - Detect duplicate entities and orphaned revision metadata.

3. **Check**
   - Apply entity-class consistency and explicit-construct validation.
   - Produce hard errors only from explicit NextTeX constructs.
   - Treat ordinary LaTeX as `UnknownOpen` (`?O`), consistent with every entity class.
   - Keep raw-LaTeX observations advisory behind `--strict-tex`.
   - Calculate checked-versus-opaque coverage. Where an author annotates fully, the useful signal is a
     coverage **drop** — something entered the document that the compiler does not understand — rather than
     an absolute threshold.

4. **Emit**
   - Erase annotations and lower native constructs to LaTeX.
   - Copy opaque spans directly from immutable source buffers.
   - Emit the mirrored `build/` tree, one `.tex` per `.ntex`.
   - Optionally produce explicitly non-transporting `--flatten` output.
   - Write source-map segments while output bytes are produced.

5. **Attribute and report**
   - Parse TeX file-line diagnostics.
   - Map output byte offsets back to source spans.
   - Assign blame as `AuthorLatex`, `NextTexNative`, `NextTexGenerated`, or `unresolved`.
   - Translate supported visual failures using declared entity names.
   - Render one diagnostic model as human-readable text, JSON, and LSP diagnostics.

### Opaque node

The opaque node is the transport boundary:

```rust
struct Opaque {
    source: SourceId,
    span: Span,
    confidence: ParseConfidence,
}
```

`Span` indexes an immutable source byte buffer. It does not hold a decoded and re-encoded copy. Emission
copies the indexed byte slice directly.

Opaque content is never normalized, expanded, structurally checked, or rejected because of unfamiliar
LaTeX. Searches for `\label`, `\ref` or `\cite` inside opaque content may produce advisory information only.

### Source map

Each emitted file has a corresponding map such as `paper.ntexmap` containing:

```rust
enum OriginKind {
    AuthorLatex,
    NextTexNative,
    NextTexGenerated,
}

struct MapSegment {
    output_start: u32,
    output_end: u32,
    origin: OriginId,
}
```

The map also records SHA-256 fingerprints of input and output; ordered, non-overlapping output segments;
source-file identities; and source spans with line indexes.

A diagnostic without a supporting map segment receives `blame: unresolved`. The compiler must not guess an
origin.

## Parser hazards

The shallow LaTeX parser preserves rather than rejects. After entering `OpaqueToEof`, it recognizes no
further NextTeX constructs.

| Construct | Parser behavior | Falsifying observation |
|---|---|---|
| `\verb`, `\verb*` | First non-space byte is the delimiter; scan literally. A missing delimiter forces `OpaqueToEof`; no delimiter is synthesized. | Bytes inside the literal are parsed as NextTeX, or an unterminated literal is repaired or rejected. |
| `verbatim`, `lstlisting` | Copy raw lines through the exact environment terminator. Extra verbatim environment names come from `nextex.toml`. | A marker inside the environment becomes a NextTeX node, or any contained byte changes. |
| `\catcode` | Do not evaluate. `OpaqueBalanced` for the remaining group only if corpus evidence establishes a reliable boundary; `OpaqueToEof` at top level. | Parsing resumes past a boundary the corpus shows can be changed by expansion. |
| `\makeatletter`, `\makeatother` | Permit `@` in control-sequence names within a matched pair. An unmatched opener makes the remainder opaque. | A control sequence in the pair is split at `@`, or native syntax is recognized after an unmatched opener. |
| `\newenvironment` and variants | Parse only the declaration shell needed to locate arguments. Bodies stay opaque; no grammar is inferred from them. | A marker in a definition body is treated as an active construct. |
| `\newcommand`, `\def`, `\edef`, `\gdef` | Record the declared control-sequence name and arity. Body stays opaque and is never expanded. | The body is expanded, normalized, or counted as checked content. |
| `\csname … \endcsname` | Preserve as an opaque balanced node. Do not infer the generated control-sequence name. | The generated name enters the symbol table as a definite declaration. |
| `\input`, `\include` | Record an opaque project edge. Do not splice file contents into the byte stream during parsing or normal emission. | Normal output contains inlined imported bytes. |
| `\if…`, `\else`, `\fi` | Preserve every branch as opaque. Do not evaluate the condition. | Only one branch is retained, or constructs in either branch produce hard errors. |

## Checking and diagnostics

Hard errors are limited to explicit NextTeX constructs:

- duplicate explicit identifiers;
- unresolved `@ref`;
- a reference requiring a known entity class that differs from the target's known class;
- unresolved files declared by typed constructs;
- `@cite` keys absent from a bibliography that was read successfully;
- unsupported length units, or percentages outside the accepted range;
- orphaned revision identifiers;
- overlapping or unbalanced revision constructs.

Ordinary `\ref`, `\cite`, unfamiliar macros, failed inference, and comparisons involving `?O` do not produce
hard errors. Optional raw-LaTeX findings are labelled `advisory`, require `--strict-tex`, and do not change
the process exit code.

Structural table checks must account for constructs such as `\multicolumn`; a simple token count is not
sufficient.

## Revision model

The source format contains:

```latex
@add(id) { text }
@del(id) { text }
@sub(id) { old -> new }
@note(id, on=entity-id) { text }
```

Metadata such as author, status, and discussion thread lives in a sidecar such as `paper.ntex.review`, keyed
by revision identifier.

The initial model forbids overlapping changes and requires balanced content. A substitution is atomic when
accepted.

Emission modes:

- `--original` — reject all pending changes;
- `--marked` — render marked revisions;
- `--final` — emit the accepted-result view.

Accepting or rejecting a change edits the source and removes its sidecar entry. `nextex check` reports
orphaned source or sidecar identifiers.

## Phases

### Phase 0 — Gather what Phase 1 needs

Two write-ups, no compiler code. Neither judges the project; both are input to the grammar and to the
diagnostics:

- rewrite a section of a published paper in the specified syntax, declaring **every** referenceable thing in
  it, and record what the language does not let you name. The output is that gap list, not a percentage.
  Every diagnostic in this project names something the author declared; what cannot be declared is what will
  never get a good error message. This does not judge the notation, which is a settled design decision;
- run a minimal defective example for every error class through `tectonic`, `chktex`, `chklref` and
  `texlab`, and record what each one reports. This is not about whether the checks are worth having — it is
  about where the messages have to be better than what LaTeX already prints, and which classes have no
  existing message at all.

**Done when** both write-ups exist: the list of things the language cannot yet name, and the table of what
existing tools already report. Both feed Phase 1 — the first says what the grammar has to cover, the second
says where the error messages need to be better than what LaTeX already prints.

### Phase 1 — Freeze the language contract

Write the formal grammar, lexical boundaries, explicit hard-error policy, erasure rules, review-mode
semantics, and representative valid and invalid examples.

The package-synthesis conflict is settled: `needs` is not a field, packages are written by the author in the
preamble, and a typed block lowers to the LaTeX its fields describe and nothing else. See
[`docs/decisions/0001`](docs/decisions/0001-typed-emission-and-no-injection.md).

**Done when** every construct has a boundary that can be found without looking ahead past end of line, and
no two examples in the spec demand different output for the same input.

### Phase 2 — Build the native language

Immutable sources, native parsing, the shared document model, project discovery, resolution, checking,
bibliography access, coverage, revision constructs, mirrored emission, source maps, and human/JSON
diagnostics. Raw LaTeX is available only through its explicit delimited escape in this phase.

**Done when** a real paper parses, checks, emits into a mirrored `build/` tree and compiles, without needing
the raw escape for ordinary figures, tables, sections and references — and every diagnostic carries a source
span and a blame value.

### Phase 3 — Add the LSP and the WASM/browser surface

Diagnostics, hover, project-wide completion, definition lookup, safe rename, and macro declaration
information through the LSP. Compile the same core to WebAssembly and connect it to browser-provided source
and output stores.

**Done when** rename across the whole project leaves no stale reference, the LSP and the CLI report the same
diagnostics for the same input, and the WASM build needs no filesystem or process access in the core.

### Phase 4 — Add the LaTeX on-ramp

Shallow LaTeX parsing, parser quarantine, opaque transport, selected macro declaration recording,
multi-file preservation, and TeX-log attribution. Assemble licensed corpus files with declared provenance
plus synthetic files for every parser hazard. Set quarantine thresholds before measuring the corpus.

**Done when** every corpus file transports byte-for-byte with no per-file special case, and quarantine stays
rare enough that real documents still have room to annotate.

### Phase 5 — Qualify the guarantees

Fuzzing for lexical boundaries and quarantine transitions, byte-transport property tests, annotation
insertion generators, deterministic TeX reruns, normalized raster comparison, and native/WASM conformance
fixtures.

**Done when** the suite is green: no byte differs on transport inputs, no valid annotation changes a rendered
page or a build status, and native and WASM agree on the shared fixtures.

## Explicitly out of scope

- A TeX interpreter or typesetting engine.
- Full TeX macro expansion.
- Evaluation of catcodes or conditionals.
- Replacing LaTeX as the artifact of record.
- Converting arbitrary LaTeX into a normalized NextTeX representation.
- Hard errors derived from ordinary unannotated LaTeX.
- Scanning opaque content and presenting matches as checked facts.
- Wildcard imports.
- Non-literal include or bibliography discovery, unless later evidence justifies it.
- Entity-specific blocks beyond demonstrated needs; `@id` remains the fallback.
- Overlapping revision changes in the initial revision model.
- A review UI before the source and sidecar data model is stable.
- Multiple TeX engines until the pinned-engine qualification path passes.
- Reuse of texlab's GPL parser in the MIT-licensed compiler.
- Claims of priority, uniqueness, or superiority.

## Not yet verified

- No compiler, LSP, native binary, WASM package, or browser integration currently exists.
- The Phase 0 syntax comparison has not been run.
- The Phase 0 tool comparison is a prediction, not a measurement.
- The formal grammar and its lexical-boundary examples are not written.
- The acceptable `OpaqueToEof` corpus threshold has not been selected.
- Group-local quarantine after `\catcode` has not been established as reliable; the safe fallback remains
  `OpaqueToEof`.
- The transport and typesetting-equivalence properties have not been exercised against a licensed corpus.
- TeX raster normalization and its noise floor have not been measured.
- The source-map format and the revision sidecar schema are not frozen.
- The interaction between typed constructs and the no-injection rule requires an explicit specification
  decision.
- The browser TeX integrations named above have not been tested with NextTeX output.
- Literature outside the languages and indexes already searched remains outside the novelty-check boundary.
- Any new external capability or priority claim requires a fresh check against sources opened for it.
