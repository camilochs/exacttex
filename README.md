# NextTeX

**Know whether your document is sound before you look at the PDF.**

NextTeX is a document language with LaTeX as its backend. Rename a `.tex` file to `.ntex` and it keeps
working — your LaTeX passes through byte for byte. From there you annotate what you want checked, and what
you annotate is guaranteed.

> **Status: design frozen, nothing built yet.** There is no working compiler in this repository. The syntax
> below is specified, not implemented. Nothing here claims to work.

---

## What it is for

Two things LaTeX cannot do today.

**Errors in your own words.** When something does not fit on the page, LaTeX says:

```
Overfull \hbox (12.3pt too wide) in paragraph at lines 45--47
```

NextTeX says:

```
your table "results" runs 12.3pt past the right margin
paper.ntex:212 — column 3 does not fit the width you declared
```

Same fact, using the name you gave the object. This works only because you declared it — which is what the
syntax is for. It gives the tooling names to speak with.

**Revisions that live in the file.** Word stores tracked changes inside the `.docx`, which is why tools can
propose edits you accept or reject. LaTeX has no equivalent, so every tool builds its own layer and none of
them interoperate. NextTeX puts the model in the format.

---

## What it looks like

```latex
\documentclass[11pt]{article}
\usepackage{amsmath}

\begin{document}
\section{Introduction} @id(sec:intro)

We argue the opposite in @ref(sec:model).
@citet(knuth1984) showed that cost grows with $n$.

The architecture is shown in @ref(fig:runtime).

\figure(fig:runtime) {
  src     = "figures/runtime.pdf"
  width   = 80%
  caption = Runtime architecture for \emph{multi-agent} systems
}

@import("sections/model.ntex")

Ordinary LaTeX keeps working: \emph{emphasis}, $E = mc^2$, \citep{blum2020}.
\end{document}
```

Two levels of annotation. `@id(x)` hangs off any LaTeX construct and buys checked references and safe rename
— theorems and algorithms work without NextTeX knowing what they are. A typed block such as `\figure(x)`
also gives the compiler the fields to check: the image resolves, the caption is there, the column count
matches the `tabular`.

---

## The type system is gradual, and that is the whole design

A document is mostly LaTeX the compiler does not model, and that is not a defect to be fixed later. It is
the state the language is built for.

Every entity is either a class the compiler knows or `?O`, the **unknown open** type:

| Class | Where it comes from |
|---|---|
| `Figure`, `Table` | a typed block — `\figure(fig:x)`, `\table(tab:x)` |
| `Section`, `Appendix`, `Algorithm`, `Equation` | `@id` attached to the LaTeX construct that is one |
| `Citation` | `@cite`, checked against the bibliography rather than against identifiers |
| `?O` | everything else |

*Open* is the load-bearing word. `?` in a gradual type system means "unknown among a fixed set of types".
LaTeX has no fixed set — any package may define new constructors at any time — so the unknown here is
unbounded. The term is from Malewski, Greenberg and Tanter, *Gradually Structured Data* (OOPSLA 2021).

Comparison is **consistency**, not equality, and two lines are the entire checking policy:

```
Known(A) ~ Known(B)   if and only if A == B     <- this can fail
?O       ~ T          for every T               <- this never fails
```

`?O` is consistent with everything, so nothing involving unmodelled LaTeX can be inconsistent, so nothing
involving unmodelled LaTeX can fail. **`?O` does not mean invalid. It means the compiler has no grounds.**

Two consequences worth stating plainly.

**Renaming a `.tex` to `.ntex` and changing nothing checks clean.** Not by care taken case by case — by
construction, because every entity in it is `?O`. That is the gradual guarantee (Siek, Vitousek, Cimini and
Boyland, SNAPL 2015) instantiated for documents, and it is what makes the on-ramp real rather than a
promise.

**You choose how much to annotate, and the compiler tells you how much you chose.** `nextex check` reports
coverage: the fraction of the document it checked. This is the analogue of `any` and `noImplicitAny`. What
matters is not the number but a fall in it — a file that was 60% checked and is now 30% gained something the
parser cannot model.

The check that types buy, and that no LaTeX tool performs: `@ref` to a `\table(fig:main)` while your prose
says "Figure". LaTeX compiles it and prints the wrong word. See [`docs/checking.md`](docs/checking.md).

---

## What it is not

It is not a shorter way to write LaTeX. TypeScript is more verbose than JavaScript; nobody adopted it to type
less. You write more so the tooling knows more.

It does not replace TeX, does not typeset, and does not ask you to leave the LaTeX ecosystem. Your journal
still receives a `.tex` file.

---

## Documentation

- [`PHILOSOPHY.md`](PHILOSOPHY.md) — what NextTeX is, what it is not, and what may be claimed. Binding.
- [`AGENTS.md`](AGENTS.md) — orientation, invariants, anti-patterns, workflow. Read before non-trivial
  changes.
- [`CONTRIBUTING.md`](CONTRIBUTING.md) — setup, tests, and the traps specific to working here.
- [`ROADMAP.md`](ROADMAP.md) — the correctness properties, the compiler architecture, the parser
  hazards, and the phases with their exit criteria.
- [`docs/grammar.md`](docs/grammar.md) — the language specification: entry tokens, constructs,
  boundaries, and where constructs are not recognized.
- [`docs/checking.md`](docs/checking.md) — what the compiler is willing to call an error, and what it
  refuses to: entity classes, the closed list of hard errors, coverage, blame, and the diagnostic fields.
- [`docs/revisions.md`](docs/revisions.md) — the change model: the four constructs, the three views,
  what accepting rewrites, and what the sidecar holds.
- [`docs/decisions/`](docs/decisions/) — accepted decisions, one file each, with the evidence behind them.
- [`docs/references.md`](docs/references.md) — what to read before each phase, and what each source
  already settled or blocked.
- [`docs/testing.md`](docs/testing.md) — the six test layers, and why the property tests cannot wait
  for the last phase.

---

## License

MIT. See [`LICENSE`](LICENSE).

An AF Labs project.
