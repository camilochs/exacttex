# Stage 0b — what existing tools say about each Class-A defect

Measured 2026-09-01 on macOS (Darwin 25.4.0); the wrong-class pair rebuilt, the prose-word pair and the
clean controls added, and every row re-measured the same day.
Tools: Tectonic 0.16.9 · ChkTeX v1.7.9 - Copyright 1995-96 Jens T. Berger Thielemann. · texlab 5.26.0 · chklref 3.1.2 · xtex-cli built from the checkout that carries this file.
Protocol: for each defect class, the minimal LaTeX carrying it (`cases/<name>/main.tex`)
ran through each tool; the annotated twin (`main.xtex`) ran through `xtex check`. Beside each pair sits
a defect-free twin (`clean.tex`, `clean.xtex`) that ran through the same five tools as a control.
Runner: `run.sh [main|clean]` (tectonic/chktex/chklref/xtex) and `texlab-probe.mjs <case> [main|clean]`
(LSP diagnostics, kept only for the opened file — a case directory now holds two roots).
chklref drives pdflatex once, with no `.aux`, and prints its own report after the engine's transcript. The
runner prints that report whole, and separately the engine's warnings and errors it relays — an earlier
runner cut the output at four lines and recorded chklref as silent.

**Prediction, registered in the plan before running** (exacttex-plan-v5 §11, 0b):
only the wrong-entity-class defect goes undetected by everything; the missing
figure file is already a hard error from TeX itself; the rest are soft warnings.
Failure criterion: four or more already detected as hard error.

## Results

| Defect | tectonic (build) | chktex | chklref | texlab (editor) | xtex check |
|---|---|---|---|---|---|
| Broken reference | exit 0, PDF ships "??", warning only in the log | silent | own report: "remove label sec:intro" (the *unused* label, not the broken reference); relays the engine's "Reference undefined", which the clean control relays too | ERROR squiggle "Undefined reference" | **hard error XT1003**, names the entity, suggests the near-miss |
| Missing citation | exit 0, "??" | style note only (`~` spacing) | own report: empty; relays "Citation undefined", which the clean control relays too | ERROR "Undefined reference" | **hard error XT1005**, names the key |
| Duplicate identifier | exit 0, silent to terminal | silent | own report: empty; relays "Label `sec:x' multiply defined", absent from the clean control | ERROR "Duplicate label" ×2 | **hard error XT1001**, points at the first declaration |
| Reference prefix demands one class, target declares another (identifier-class mismatch) | **undetected** | **undetected** | **undetected** (relays only the first-pass "undefined", as on the clean control) | **undetected** | **hard error XT1004**: "requires figure, but its target is table", declaration linked |
| Prose says Figure, target is a table (prose-word mismatch) | **undetected** | **undetected** | **undetected** (relays only the first-pass "undefined", as on the clean control) | **undetected** | **hard error XT1020**: "prose says `Figure` but `tab:main` is a table", declaration linked |
| Missing figure file | **hard error** (TeX itself) | silent | own report: empty; relays "! Package pdftex.def Error: File not found", absent from the clean control | silent | **hard error XT1006**, before TeX runs |
| Invalid unit | **hard error** (TeX: "Illegal unit") | silent | own report: empty; relays "! Illegal unit of measure", absent from the clean control | silent | **hard error XT1007**, names the field, lists valid shapes |

Two rows, two defects, each a matched pair. The wrong-class pair encodes the prefix defect: `main.tex` is a
table labelled `fig:main` and referenced as `Table~\ref{fig:main}`; `main.xtex` is `\table(fig:main)`
referenced as `Table~@ref(fig:main)`. The prefix `fig:` demands a figure, the declaration is a table, and
that contradiction is `XT1004` (`docs/decisions/0003`). The prose-word pair encodes the defect the sentence
makes: `main.tex` is a table labelled `tab:main` and referenced as `Figure~\ref{tab:main}`; `main.xtex` is
`\table(tab:main)` referenced as `Figure~@ref(tab:main)`. The word says figure, the declaration says table,
and that contradiction is `XT1020` (`docs/decisions/0019`). An earlier version of this file had one row
comparing the two defects with each other: the `.tex` carried the prose-word mismatch and the `.xtex` the
prefix mismatch. When that was corrected, the prose-word defect was caught by no tool in this table, xtex
included; `XT1020` was then added as a language feature, and this row is its measurement.

## Clean controls

Each `clean.*` twin is the same document with the defect removed: the reference resolves, the key is in
`refs.bib`, the labels differ, the prefix matches the declaration, the word matches the declaration, the
figure file exists (a 724-byte PDF under `figures/`), the unit is `cm`. Expected: zero diagnostics
everywhere. The wrong-class and prose-word controls are the same document — `\table(tab:main)` referenced
as `Table~@ref(tab:main)` — because removing either defect lands on it.

| Case | tectonic | chktex | chklref | texlab | xtex check |
|---|---|---|---|---|---|
| broken-ref | exit 0 | silent | own report: empty; relays first-pass "Reference `sec:intro' undefined" | silent | exit 0, no diagnostics |
| missing-cite | exit 0 | style note (`~` spacing), the same one as on the defective twin | own report: empty (the uncited `real1984` is not listed either); relays first-pass "Citation `real1984' undefined" | silent | exit 0, no diagnostics |
| duplicate-label | exit 0 | silent | own report: empty; relays first-pass "undefined" for both labels | silent | exit 0, no diagnostics |
| wrong-class | exit 0 | silent | own report: empty; relays first-pass "Reference `tab:main' undefined" | silent | exit 0, no diagnostics |
| prose-word | exit 0 | silent | own report: empty; relays first-pass "Reference `tab:main' undefined" | silent | exit 0, no diagnostics |
| missing-figure | exit 0 | silent | own report: empty; relays nothing | silent | exit 0, no diagnostics |
| invalid-unit | exit 0 | silent | own report: empty; relays nothing | silent | exit 0, no diagnostics |

What actually happened, against the expectation of silence: tectonic, texlab and xtex are silent on all
seven. chktex's one note is a spacing style rule that fires on `\cite` with or without the defect. chklref's
own report is empty on all seven, but the engine warnings it relays are not: every clean document that
contains a `\ref` or `\cite` is reported "undefined" on that single pass, because chklref runs pdflatex
once with no `.aux` and the engine cannot resolve anything on a first pass. A relayed "undefined" is
therefore not evidence about the document. The relayed lines that *do* separate defect from control are
the three the engine produces on a first pass regardless of `.aux`: "multiply defined", "File not found"
and "Illegal unit".

## Reading

- **The prediction held on its core claim**: the identifier-class mismatch is invisible
  to every existing tool — it is exactly the check that typed declarations buy. The
  row names the defect honestly, and the prose-word defect has its own row: invisible to
  every existing tool as well, and caught by xtex only since `XT1020` made the word before a
  reference a checked side (`docs/decisions/0019`). Before that, this experiment's own result
  file had described the prose-word defect as caught when it was not.
- **The prediction was wrong on one count, recorded as such**: the invalid unit
  is a HARD TeX error, not a soft warning. Hard-error count in existing tools: 2
  (< 4), so the stage's failure criterion is not met.
- **chklref was under-read, not silent.** The first runner cut its output at four lines, before
  its report. Read whole, its own report finds one thing across the six defects — an unused label — and
  what it relays from the engine is mostly first-pass noise; the clean controls are what showed which
  relayed lines carry information (three of them) and which do not (every "undefined").
- **texlab is the strongest incumbent**: three defects get ERROR-severity editor
  squiggles, and its clean controls are silent. But nothing gates: no exit code, no build refusal — and the messages
  are generic ("Undefined reference") where xtex names the entity, its class,
  and the declaration site.
- **The build tool is the quietest**: with a broken reference, tectonic exits 0,
  prints progress notes to the terminal, ships a PDF containing "??", and the
  only trace of the problem is one line inside main.log.
- **No false positives on the controls** from the two checkers that gate or squiggle: xtex and texlab
  report nothing on any clean twin. The claim rests on seven minimal documents and says nothing about
  larger ones.
