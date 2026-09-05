<p align="center">
  <img src="docs/assets/exacttex-logo.svg" alt="ExactTeX" width="420">
</p>

<p align="center"><strong>Know whether your document is sound before you look at the PDF.</strong></p>

<p align="center">
  <a href="https://github.com/camilochs/exacttex/actions/workflows/rust.yml"><img src="https://github.com/camilochs/exacttex/actions/workflows/rust.yml/badge.svg" alt="CI"></a>
  <img src="https://img.shields.io/badge/license-MIT-5a23ee" alt="MIT">
  <img src="https://img.shields.io/badge/dependencies-0-082768" alt="zero dependencies">
  <a href="https://github.com/camilochs/exacttex/releases"><img src="https://img.shields.io/github/v/release/camilochs/exacttex?label=release&color=082768" alt="latest release"></a>
  <img src="https://img.shields.io/badge/rust-1.88%2B-5a23ee" alt="Rust 1.88 or newer">
  <a href="https://camilochs.github.io/exacttex/"><img src="https://github.com/camilochs/exacttex/actions/workflows/docs.yml/badge.svg" alt="docs"></a>
</p>

ExactTeX is LaTeX with gradual annotation. You name the objects you want checked; everything else stays
ordinary LaTeX and is copied through byte for byte. Rename a `.tex` file to `.xtex` and it still compiles.
Annotate as much or as little as you want. What you annotate is checked.

You can try it in the browser at [Vitela](https://vitela.artificialfallibility.com/), an editor built on
the compiler's WebAssembly build. The checks run locally in the page.

---

## What it does

**Errors in your own words.** When a table is too wide, LaTeX says:

```
Overfull \hbox (12.3pt too wide) in paragraph at lines 45--47
```

ExactTeX says:

```
your table "results" runs 12.3pt past the right margin
paper.xtex:212 — column 3 does not fit the width you declared
```

Same fact, with the name you gave the table. That is what the annotations are for: they give the tooling
names to use.

**Revisions inside the file.** Word keeps tracked changes inside the `.docx`, so other tools can propose
edits you accept or reject. LaTeX has nothing like it, and every tool builds its own incompatible layer.
ExactTeX puts the change model in the file format.

---

## Try it

```sh
git clone https://github.com/camilochs/exacttex
cd exacttex
cargo run -p xtex-cli -- check examples/hello.xtex
```

```
coverage: 11.8%
bibliography: unavailable — the document declares no bibliography
```

The file is ordinary LaTeX with one section annotated. `coverage` is the share of the document under
check. Now misspell the reference: change `@ref(sec:results)` to `@ref(sec:resutls)` and check again.

```
error[XT1003]: identifier `sec:resutls` is not declared — did you mean `sec:results`?
  --> examples/hello.xtex:6:30
  entity: section
  name: sec:resutls
  span: offset 106, length 11
  --> examples/hello.xtex:4:22: `sec:results` is declared here
  blame: xtex-construct
```

Plain LaTeX would have typeset a `??` and said nothing. You need a [Rust toolchain](https://rustup.rs)
(1.88 or newer). The compiler has no dependencies to fetch.

## What it looks like

```latex
\documentclass[11pt]{article}
\usepackage{amsmath}

\begin{document}
\section{Introduction}@id(sec:intro)

We argue the opposite in Section~@ref(sec:model), and
@cite(knuth1984) showed that cost grows with $n$.

The architecture is shown in Figure~@ref(fig:runtime).

\figure(fig:runtime) {
  src     = "figures/runtime.pdf"
  width   = 80%
  caption = {Runtime architecture for \emph{multi-agent} systems}
}

@import("sections/model.xtex")

Ordinary LaTeX keeps working: \emph{emphasis}, $E = mc^2$, \citep{blum2020}.
\end{document}
```

There are two levels of annotation. `@id(x)` attaches to any LaTeX construct and gives you checked
references and safe renames; theorems and algorithms work without ExactTeX knowing what they are. A typed
block such as `\figure(x)` also gives the compiler fields to check: the image exists, the caption is
present, the column count matches the `tabular`.

---

## The type system is gradual

Most of a document is LaTeX the compiler does not model. That is the normal state, and the language is
built for it.

Every entity is either a class the compiler knows or `?O`, the unknown open type:

| Class | Where it comes from |
|---|---|
| `Figure`, `Table` | a typed block: `\figure(fig:x)`, `\table(tab:x)` |
| `Section`, `Appendix`, `Algorithm`, `Equation` | `@id` attached to the LaTeX construct |
| `Citation` | `@cite`, checked against the bibliography |
| `?O` | everything else |

`xtex inventory paper.xtex` prints this table for your document: one line per identifier, with its class,
how many references use it, and where it was declared.

"Open" matters. In a gradual type system `?` means "unknown among a fixed set of types". LaTeX has no fixed
set, because any package can define new constructors, so the unknown here is unbounded. The term comes from
Malewski, Greenberg and Tanter, *Gradually Structured Data* (OOPSLA 2021).

Comparison is consistency rather than equality. The whole checking policy is two lines:

```
Known(A) ~ Known(B)   if and only if A == B     <- this can fail
?O       ~ T          for every T               <- this never fails
```

`?O` is consistent with everything, so nothing that involves unmodelled LaTeX can fail. `?O` means the
compiler has no grounds to judge, and nothing more.

Two consequences follow.

Renaming a `.tex` to `.xtex` and changing nothing checks clean, because every entity in it is `?O`. This is
the gradual guarantee (Siek, Vitousek, Cimini and Boyland, SNAPL 2015) applied to documents, and it is why
the on-ramp works.

You choose how much to annotate, and the compiler reports how much you chose. `xtex check` prints coverage,
the fraction of the document it checked. It is the analogue of `any` and `noImplicitAny`. The trend
matters more than the number: a file that went from 60% to 30% gained something the parser cannot model.

One check that types make possible and no LaTeX tool performs: `@ref(fig:main)` pointing at a
`\table(fig:main)`. The prefix says figure; the declaration says table. Likewise `Figure~@ref(tab:main)`
on a table: the sentence says figure, the declaration says table. LaTeX compiles both and prints the wrong
word. See [`docs/checking.md`](docs/checking.md) and
[`docs/decisions/0019`](docs/decisions/0019-prose-is-a-checked-side.md).

---

## How it is built

![ExactTeX architecture](docs/assets/architecture.svg)

One core with no dependencies. Three thin surfaces (terminal, editor, browser) call it, and a parity suite
in CI keeps their output byte-identical. Unannotated bytes never enter the pipeline; they are carried
around it. The full description is in [docs/architecture.md](docs/architecture.md).

External verification (bibliography entries, URLs, DOIs and repositories checked against live sources) is a
separate step. It writes a dated record, and the compiler replays that record offline, so a compile never
touches the network. See [docs/verification.md](docs/verification.md).

---

## What it is not

It is not a shorter way to write LaTeX. TypeScript is more verbose than JavaScript, and nobody adopted it to
type less. You write more so the tooling knows more.

It does not typeset and it does not replace TeX. Your journal still receives a `.tex` file.

---

## How early this is

The compiler is still young. And yes, I built it with agents; without them, it would have taken months. The design, however, is mine.

The transport guarantee (untouched LaTeX comes out byte-identical) is the oldest and most tested invariant.
The checker, the emitter, the WebAssembly build and the LSP give the same answer for the same input, and CI
enforces it. A book of mine (100+ pages, around forty packages, TikZ, an index, per-chapter
bibliographies) compiles to the same page count as with a full TeX Live.

The change model is newer. Three bugs found in real use in August 2026 had one cause: two code paths
disagreed about where a document carries text. All three are fixed, each with a regression test.

Short version: document transport is mature; collaborative revision is newer. If you find a bug, send me
the document.

---

## Documentation

<https://camilochs.github.io/exacttex/> has the language specification, what the compiler calls an error,
the change model, the WebAssembly and LSP surfaces, and every accepted decision with its evidence. The same
pages live in [`docs/`](docs/), one Markdown file each.

Two documents bind anyone changing the code: [`PHILOSOPHY.md`](PHILOSOPHY.md) (what may be claimed) and
[`AGENTS.md`](AGENTS.md) (invariants and workflow).

---

## License

MIT. See [`LICENSE`](LICENSE).

An [AF Labs](https://labs.artificialfallibility.com/) project.
