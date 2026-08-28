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
@cite(knuth1984, style=textual) showed that cost grows with $n$.

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
buys visual checks and writes the packages it needs into the preamble for you.

---

## What it is not

It is not a shorter way to write LaTeX. TypeScript is more verbose than JavaScript; nobody adopted it to type
less. You write more so the tooling knows more.

It does not replace TeX, does not typeset, and does not ask you to leave the LaTeX ecosystem. Your journal
still receives a `.tex` file.

It does not compete with [Typst](https://typst.app), which asks an author to leave their corpus behind.
NextTeX asks you to keep it.

Prior art it does **not** claim to have invented: typed document languages that emit LaTeX
([MyST](https://mystmd.org)), semantic annotation over LaTeX with editor support
([sTeX](https://ctan.org/pkg/stex)), and language servers for LaTeX
([texlab](https://github.com/latex-lsp/texlab), [digestif](https://github.com/astoff/digestif)).

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
- [`docs/references.md`](docs/references.md) — what to read before each phase, and what each source
  already settled or blocked.
- [`docs/testing.md`](docs/testing.md) — the six test layers, and why the property tests cannot wait
  for the last phase.

---

## License

MIT. See [`LICENSE`](LICENSE).

An AF Labs project.
