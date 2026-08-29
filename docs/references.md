# References, by phase

What to read before each phase, why, and what decision it bears on. Findings recorded here were obtained by
reading the source or the specification — not the description of it. Where only an abstract was read, it says
so.

Where a technique is published, this document carries the technique and the clone is disposable. Where it is
not — TexSoup has no paper — the source is the only statement of it, and the findings below come from reading
it. Any cloned source lives outside this repository, under `references/` in the project directory, so
third-party code is never vendored here by accident.

---

## Phase 1 — Freeze the language contract

### TypeScript Design Goals

<https://github.com/microsoft/TypeScript/wiki/TypeScript-Design-Goals> · read in full

The canonical statement of the strategy ExactTeX copies. Five of its goals and two non-goals are, near
word-for-word, rules this project derived independently:

| TypeScript | ExactTeX |
|---|---|
| "Impose no runtime overhead on emitted programs" | erasure, never injection (`PHILOSOPHY.md` §5) |
| "Emit clean, idiomatic, recognizable JavaScript code" | the emitted `.tex` is meant to be read and submitted |
| "Preserve runtime behavior of all JavaScript code" | typesetting equivalence, property B |
| "Use a consistent, fully erasable, structural type system" | annotations erase to nothing |
| *Non-goal:* "Apply a sound or 'provably correct' type system" | the TypeScript side of the gradual fork, not Typed Racket |
| *Non-goal:* "Add or rely on run-time type information" | no support package, no assertions in output |

**Bearing.** Use this wording when specifying erasure. It is settled prior art, and matching it makes the
design legible to anyone who knows TypeScript.

### Gradual typing — the theory

- Siek, Vitousek, Cimini, Boyland, *Refined Criteria for Gradual Typing*, SNAPL 2015 — the gradual guarantee,
  which is the correctness specification in `ROADMAP.md`.
- Malewski, Greenberg, Tanter, *Gradually Structured Data*, OOPSLA 2021 — `?O`, the unknown **open** datatype.
  LaTeX's macro universe is open: any package defines new constructors, so ordinary LaTeX is `?O`, not `?`.
- Crichton & Krishnamurthi, *A Core Calculus for Documents*, POPL 2024 (arXiv 2310.04368) — the boundary
  between document content and computation. **Abstract and metadata only; the body has not been read.** No
  theorem is attributed to it.

---

## Phase 2 — Build the native language

### rowan — lossless syntax trees

<https://github.com/rust-analyzer/rowan> · cloned · MIT **and** Apache-2.0 · ~3 600 lines
Design overview: <https://rust-analyzer.github.io/book/contributing/syntax.html> · read

**The technique, stated so the crate is not needed to implement it.** A syntax tree is split in two layers
that hold different things:

- **Green tree — the data.** A purely functional n-ary tree. Each node holds a kind tag, its total text
  length, and `Arc` pointers to children; each leaf holds the token's full text. It has **no parent
  pointers** and no absolute positions, which is exactly what lets identical subtrees be shared by pointer:
  a node knows what it contains, not where it is. Losslessness follows by construction rather than by
  discipline — whitespace and comments are tokens like any other, so the tree cannot fail to reproduce its
  input.
- **Red tree — the cursor.** A thin wrapper created on demand while walking down from the root. It carries a
  pointer to a green node, a pointer to its parent cursor, and its absolute offset — computed by summing the
  lengths of preceding siblings. It is cheap to clone by refcount and gives identity semantics, so two
  occurrences of the same shared green subtree are distinguishable.

The split exists because those two needs conflict: sharing requires position-independence, and navigation
requires position. Editing is roughly O(depth) because only the spine from the changed node to the root is
rebuilt; everything else is shared. That is what makes incremental reparsing viable.

Two stated invariants, both of which ExactTeX wants verbatim:

> "Parsing is lossless (even if the input is invalid, the tree produced by the parser represents it exactly)."
> "Parsing is resilient (even if the input is invalid, parser tries to see as much syntax tree fragments in
> the input as it can)."

Modifying the tree is roughly O(depth), which is what makes incremental reparsing viable for the LSP.

**Finding that blocks adopting it as-is.** The public API is UTF-8. `GreenToken::new(kind, text: &str)` takes
a `&str`, and `text()` reads back with `std::str::from_utf8_unchecked`. The *storage* is bytes
(`ThinArc::from_header_and_iter(head, text.bytes())`), but nothing in the API admits a non-UTF-8 byte
sequence. A `.tex` in Latin-1, or carrying stray bytes from an old package, cannot round-trip through it
without either transcoding — which breaks transport byte-equality — or constructing green tokens by a route
the crate does not offer.

**How much that constraint costs, measured.** Across 111 `.tex` files from six published papers, **111 are
valid UTF-8 and none are not**. The constraint is real in principle and so far unobserved in practice.
Caveat: that is one author's recent corpus. Older documents, files from coauthors, and some publisher
templates are where Latin-1 appears, and the Phase 4 corpus — which includes third-party files — is where
this gets tested properly.

**Bearing — repository issue #8.** Four options. Adopt rowan and require UTF-8 input; fork it with a
byte-oriented token; reimplement green/red byte-oriented, which is a published design and not a licensing
question; or keep a span-into-immutable-buffer node and revisit at Phase 3.

The weight is not licensing — rowan is dual MIT/Apache and can simply be a dependency. It is where rowan's
cost sits: **1 484 of its 2 332 lines are the cursor layer**, which buys incremental reparsing and cheap
navigation. Phase 3 wants that; no Phase 2 issue asks for it.

Whichever is chosen, one condition binds: the node interface must allow the representation to be swapped.
Otherwise Phase 3 stops being "add the LSP" and becomes a core refactor — the deferred-cost trap of issue #7.

### Source maps — ECMA-426

<https://tc39.es/ecma426/> · read

The standard ExactTeX's `.xtexmap` is reinventing. Three points that bear on the design:

- **Mappings are points, not ranges.** A segment gives a generated column and optionally a source position;
  the region it covers is implied by the next segment. `ROADMAP.md` currently specifies explicit
  `{output_start, output_end}` ranges.
- **Generated code with no source already has a slot.** A one-field segment "represent[s] generated code that
  is unmapped because there is no corresponding original source code, such as code that is generated by a
  compiler" — which is exactly `NextTexGenerated`.
- **Ordering is normative.** Mappings sort ascending by generated position, matching the ordered,
  non-overlapping requirement already in the plan.

**Bearing — repository issue #14.** Ranges are more direct for blame lookup than points, and ExactTeX is not
obliged to emit a JavaScript source map. But if an editor should ever consume the map, format compatibility
stops being free. Decide deliberately rather than by omission.

---

## Phase 3 — LSP and the browser surface

- **TeX in WebAssembly.** SwiftLaTeX compiles XeTeX and pdfTeX to WASM and runs them entirely
  client-side (<https://github.com/SwiftLaTeX/SwiftLaTeX>). TeXlyre-BusyTeX supports TeX Live 2026 with
  pdfLaTeX, XeLaTeX and LuaLaTeX plus BibTeX and makeindex
  (<https://github.com/TeXlyre/texlyre-busytex>). **Not tested against ExactTeX output.**
- **LSP prior art in the same domain:** texlab (<https://github.com/latex-lsp/texlab>, Rust, **GPL-3.0** — do
  not copy code into this MIT project) and digestif (<https://github.com/astoff/digestif>).

---

## Phase 1 and 4 — Specification, where one exists

There is no grammar of LaTeX to read, and there cannot be one in the usual sense: TeX is a macro expansion
engine, not a language with a context-free syntax, and the character categories that stand in for a lexer are
mutable at runtime. `tex.web` defines a machine, not a grammar. LaTeX on top of it is a macro package, and
packages redefine its commands.

Three specifications do exist and replace guesswork:

- **The category code table.** Sixteen categories, documented, and the actual lexical specification for TeX's
  default state. TexSoup's `category.py` is a hardcoded snapshot of it. Encode it; do not infer it.
- **`xparse` argument specifications.** LaTeX's own declarative notation for what arguments a command takes —
  `m` mandatory, `o` optional, and so on. The closest thing to a grammar LaTeX offers, and it is official.
- **unified-latex's macro signature database**
  (`packages/unified-latex-ctan/package/*/provides.ts`) — those signatures for 18 CTAN packages, including
  `latex2e`, `tikz`, `hyperref`, `listings`, `xcolor`, `tabularx`, `cleveref` and `mathtools`. This answers
  which commands take arguments, which is what "argument position" means in `grammar.md` §8.

**A specification says what is legal; the corpus says what is common.** Both are needed and they answer
different questions. That 442 of 479 labels sit on the line after their caption is not in any specification —
it is how people write. Which commands take arguments is in one, and measuring it instead was wasted work.

---

## Phase 4 — The LaTeX on-ramp

### TexSoup — a shallow LaTeX parser that works

<https://github.com/alvinwan/TexSoup> · cloned · ~3 700 lines Python
Benchmark: parsed 50/50 papers in an arXiv AI/ML set with a 10 s timeout; LaTeXML 29/50, plasTeX 11/50.
*(Number from a cited benchmark, not our own run.)*

Two findings from reading the source, both directly relevant:

**It does not implement `\catcode` reassignment at all.** `TexSoup/category.py` holds a module-level constant
`CATEGORY_CODES` mapping characters to TeX category codes, and `categorize()` is a flat lookup over it.
Several codes are marked `# not used` (alignment, macro parameter, superscript, subscript, active). There is
no code path anywhere that changes a category at runtime.

This is evidence, not permission. TexSoup succeeds on real papers while ignoring the hazard our plan treats
as worst-case, which suggests catcode reassignment is rare in practice — but TexSoup has no byte-transport
invariant to uphold, so "useful" is a lower bar than "correct". **The right conclusion is that the frequency
is measurable, and Phase 4 should measure it in our own corpus before defaulting every `\catcode` to
`OpaqueToEof`.**

**Its verbatim list is a better default than ours.** `TexSoup/tokens.py` line 15:

```python
SKIP_ENV_NAMES = ('lstlisting', 'verbatim', 'verbatimtab', 'Verbatim', 'listing')
```

`Verbatim` (capitalised, from `fancyvrb`), `verbatimtab` and `listing` were not in our list. Use this as the
seed for the `xtex.toml` default.

### Other shallow parsers, not read

- **unified-latex** (<https://github.com/siefkenj/unified-latex>) — TypeScript, PEG grammar plus
  post-processing keyed on known macro behaviour. Its README states parsing "should work on your code, unless
  you do complicated things like redefine control sequences". Described, not read.
- **LaTeXML** — Perl, mature, 29/50 on the benchmark above. Not read.

---

## Phase 5 — Qualification

Nothing read yet. The open questions are the raster-comparison noise floor and deterministic TeX
reproduction, and both are measurements rather than literature.

---

## Positioning — not tied to a phase

The novelty check of 2026-08-28 established what may not be claimed. Closest prior art, by strategy:

- **MyST / mystmd** (<https://mystmd.org>) — typed directives, labelled cross-references, emits LaTeX, and
  parses `.tex` via `@unified-latex`. Its own documentation calls the LaTeX path "a transitional solution"
  and states it is not a full LaTeX renderer; a broken cross-reference "raise[s] a warning". It **converts**;
  ExactTeX **transports**.
- **sTeX / sTeXIDE** (arXiv 1005.5489, 1010.5935) — semantic macros inside a LaTeX superset, with an AST,
  semantic tagger, consistency validator and context-aware completion. Mathematical semantics, not document
  structure; Eclipse-based, 2010.

Boundary of that check: no literature in German or Chinese, no sweep of CTAN. sTeX comes out of Erlangen, so
its ecosystem is the most likely place for something the check missed.

### What breaks when LLMs write LaTeX

- **TeXpert** (arXiv 2506.16990, Kale & Nadadur) — formatting and **package** errors are the dominant failure
  class, and accuracy drops sharply as task complexity rises. This is the evidence behind putting package
  requirements next to the construct that needs them.
- **Liu et al.**, *LaTeX Compilation: Challenges in the Era of LLMs* (arXiv 2603.02873) — names error
  localization as one of four fundamental TeX defects. Their conclusion is to leave TeX behind; ours is to
  keep it.

### Agent-authored revision, already occupied

Claude for Word (tracked changes as the native Word data model), Microsoft's Legal Agent, ADEU,
R3 (arXiv 2204.03685), PaperJury (arXiv 2606.16322) and PatchWrite (arXiv 2608.23001). The last two were read
in full: both are pipelines over `.tex`, neither defines syntax or a format, and PatchWrite explicitly
excludes tracked changes and figure/table reference validation.
