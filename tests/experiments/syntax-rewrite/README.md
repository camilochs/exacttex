# Phase 0a — rewriting a published paper, and what the language could not name

Run 2026-08-30, on the author's published BRKGA+LLM paper (`sn-article.tex`, 1,865 lines, 14 figures,
23 tables, 82 references, 62 citations). Every `\label`, `\ref` and `\cite` was converted mechanically to
`@id`, `@ref` and `@cite` — 196 constructs — and the whole file checked and emitted.

## The criterion, restated by the director before the numbers were read

**"No queremos escribir menos, queremos ser más legible y más seguro."** The original gate counted
delimiters and lines; that measured the wrong quantity, the same failure mode Gate 0b's threshold had. The
numbers are recorded anyway, and they show why brevity was never the pillar: 196 constructs, −392 braces,
−150 bytes — net writing effort is unchanged. TypeScript is the precedent: annotations *add* characters and
won on the guarantee.

What the rewrite measures under the corrected criterion is the **gap list** — everything the paper
declares that the language cannot see — because every diagnostic names something the author declared, and
what cannot be declared can never get a good error.

## The gap list

1. **Placement is unspeakable in a typed block.** `\figure(id){...}` emits `\begin{figure}` with no
   `[ht]`. Measured over the author's corpus: **634 of 662** figure/table environments carry explicit
   placement (96%) — `[t]` alone appears 247 times. A typed block that drops placement cannot express the
   corpus's normal case, so annotated figures stay in the environment form and the block form goes unused.

2. **A label declared as a package option is invisible.** `\begin{lstlisting}[label={lst:x}]` declares
   `lst:x`; there is no `\label` to convert. An `@ref(lst:x)` is then a hard error against a correct
   document. The paper has one; any listings-heavy paper has many.

3. **A construct inside a known command's argument is silently inert.** `@ref(sec:s)` inside `\caption{}`,
   `\footnote{}`, `\textbf{}` is ordinary bytes under `grammar.md` §8 ("Command argument" row) — *and is
   emitted literally into the PDF, with exit 0 and no advisory*. The specified behaviour and the safe
   behaviour disagree: a reader of the PDF sees `@ref(sec:s)` in a caption. The paper hit this once,
   mechanically, on its first conversion. This is the highest-risk row of the list because it is silent in
   exactly the place an author will not look.

4. **Multi-key citation spacing is normalised.** `\cite{a, b}` → `@cite(a, b)` → `\cite{a,b}`. Renders
   identically; byte-level transport of the *annotated* construct is not promised. Recorded, not a defect.

## What held

The converted paper checks clean (the two package-option labels excepted, per gap 2) and every reference
and citation resolves against the real `.bib`. The conversion is mechanical — a regex, not judgement —
which is itself evidence for the on-ramp: 196 constructs entered a real paper without reading it.

## Boundary

One paper, one author, mechanical conversion. The gap list is what this run surfaced, not a claim of
completeness — the roadmap's own instruction for 0a ("record what the language does not let you name") is
satisfied by the list existing and feeding grammar decisions, not by the list being finished.
