# Adopting a LaTeX project

`xtex adopt` turns a `.tex` project into an ExactTeX one, mechanically, and checks its own work before
writing a byte. It is the ramp the corpus experiment measured by hand over 50 arXiv papers — 221 files,
emission identical to the original except the `.tex` an import writes back — moved into the compiler, so
the CLI, the browser and an editor run one implementation.

```sh
xtex adopt paper.tex               # writes paper.xtex beside paper.tex, and one .xtex per \input it converts
xtex adopt --in-place paper.tex    # the same, then removes each .tex it converted
xtex adopt --json paper.tex        # the report as JSON
```

The project root is the directory holding the file named. Everything the command writes is reported; a
file it does not write is reported with the reason.

---

## What it rewrites

Level 1 only: the constructs an author would write with `@`, where the compiler can then check them, and
nothing whose emitted shape is its own. Every rule below applies in **live text only** — the regions
[`grammar.md`](grammar.md) §8 does not exclude. A `\cite` in a comment, a `\verb`, a verbatim or listing
body, the `comment` package's environment, an `\iffalse … \fi`, inline or display math, a `\newcommand`
body or the data argument of a command stays as written, because a construct there would ride into the PDF
as literal text. The scanner decides both, so the two cannot disagree.

| Written | Becomes | Left as written when |
|---|---|---|
| `\cite{k1,k2}`, `\citep`, `\citet`, `\textcite`, `\parencite` | `@cite(k1,k2)`, `@citep(…)`, … | a star or an optional argument is present; a key falls outside the grammar's `bibkey`; the list carries a space after a comma, which the emitter would not write back |
| `\label{x}` | `@id(x)` | `x` falls outside the grammar's `ident`; an optional argument is present |
| `\ref{x}` | `@ref(x)` | as `\label`; a star is present. Other reference commands are not rewritten |
| `\input{p}`, root file only | `@import("p.xtex")`, and `p.tex` is converted too | no `p.tex` exists beside the root; the path carries `\`, `#` or `$`, a quote, a parenthesis or edge spaces; the target file did not pass the guarantee. `\include` is never rewritten, and an `\input` inside an imported file stays |

One opening into math: the header slot of a display-math environment (§4). `\begin{equation}` followed by
whitespace of at most two line endings and then `\label{eq:x}` becomes `@id(eq:x)`, which is where the
compiler reads it. A `\label` deeper in the body stays, and is still inventoried as an equation.

## The guarantee

After converting a file the command emits it — the same emission `xtex build` writes from the project root,
original view — and compares it with the original file byte for byte. The one admitted difference is the
extension: `\input{sections/a}` was rewritten to `@import("sections/a.xtex")`, which emits
`\input{sections/a.tex}`, and TeX reads both the same way. Anything else that differs leaves the file as it
was, and the report says at which byte.

Three consequences, all deliberate:

- **A file that fails is not written, and nothing of it is written.** There are no partial conversions.
- **A root that fails leaves the whole project as it was.** Its imports may have passed on their own, but
  with the root untouched a renamed child breaks the author's build.
- **A child that fails keeps its `\input` in the root.** The root still converts; that one edge stays LaTeX.

A file that already carries an ExactTeX construct is left too: its emission is not its own bytes, so the
guarantee cannot be stated for it.

The command never overwrites a file. Not a `.tex`, and not a `.xtex` either — an earlier run's output is
somebody's edited file by now. Every output is checked for absence before any is written. `--in-place`
removes each `.tex` after its `.xtex` is written: a rename, done in the order that cannot lose the
original.

## The report

```
main.tex -> main.xtex: 2 citations (3 keys), 3 ids, 2 refs, 2 imports
sections/intro.tex -> sections/intro.xtex: 1 citations (1 keys), 1 ids, 1 refs, 0 imports
  left: sections/intro.tex:3:16 \input{sections/deeper} — \input is converted in the root file only
sections/method.tex -> sections/method.xtex: 0 citations (0 keys), 1 ids, 1 refs, 0 imports
  left: sections/method.tex:2:4 \citep[see][p.~3]{lamport1994} — a star or an optional argument keeps the citation as written
3 of 3 files converted
```

Every construct in live text that the rules did not rewrite is listed with its position and the reason.
Constructs the rules never look at — `\citealp`, `\cref`, `\eqref` — are not listed; they are ordinary
LaTeX and stay so. `--json` prints the same report as one object: `root`, then `files`, each with `file`,
`output`, `converted`, the five counts, `left` (line, column, the construct, the reason) and, for a file
that was left, `failure`.

Exit code 0 when every file passed, 1 when any file was left, 2 for a usage or I/O error. The output of
`xtex adopt` is what the fixture directory `tests/fixtures/adopt/` pins, one rule or exclusion per case,
and `crates/xtex-core/tests/adopt.rs` re-checks the guarantee over every case through the public emitter,
without the command's own gate.

## What it does not do

It does not check. `xtex check` on the result may report a duplicate `\label` the paper already had, a
`\ref` to nothing, or a key absent from the bibliography — that is the point of the conversion, and the
command does not revert a construct to hide a defect the author has.

It does not convert figures and tables into typed blocks. A block's emitted shape is its own
(`\centering`, option order, the `\label` after the caption), so the guarantee there is Property B — the
same rendered page — not the same bytes. That is level 2, opt-in per float, and a later issue.

In the browser the same function is `xtex_adopt` ([`wasm.md`](wasm.md)); the host writes the files the
answer carries and keeps or removes the `.tex` as the CLI's two modes do.

## Reproducing the corpus comparison

The E2 twins are the reference this command was built against. `tests/corpus/adopt-twins.py` runs the
command over each corpus paper and compares every output with the experiment's converted file:

```sh
cargo build --release -p xtex-cli
python3 tests/corpus/adopt-twins.py --xtex "$PWD/target/release/xtex" \
    --papers CORPUS/papers --manifest CORPUS/manifest.csv \
    --twins E2/work-69338b6 --results E2/out-69338b6 --scratch /tmp/adopt-twins
```

Over the 50 papers on 2026-09-02: 201 of 221 files byte-identical to the twin; 13 differ only where the
experiment's script reverted a construct after `xtex check` reported it (59 reverts, each matched to the
experiment's own record); 5 differ only because the script normalised CRLF line endings to LF, which the
command does not do; 2 differ where the script's regular-expression scanner read a `$` as opening math
inside `\AxiomC{$$}` and inside `\lstinline|$loc|`, and left the text after it unconverted. Every
difference is classified by the harness, and an unclassified one fails the run.
