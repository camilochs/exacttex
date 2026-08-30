# ExactTeX — Philosophy

This document is binding for anyone working on ExactTeX, human or agent. Read it before writing code,
documentation, commit messages, README copy, or a paper abstract. It exists because the same wrong framing
was proposed three times during design, each time plausibly.

---

## 1. What ExactTeX is

**ExactTeX gives a writer information about the reliability of their document before they look at the PDF.**

It is a one-directional superset of LaTeX. Every valid `.tex` file is valid ExactTeX input. A `.xtex` file does
not have to compile under plain TeX — a `.ts` file does not run in node either.

LaTeX remains the backend and the artifact of record. ExactTeX does not typeset, does not replace TeX, and
does not ask anyone to leave the LaTeX ecosystem.

## 2. What ExactTeX is not

**It is not a shorter way to write LaTeX.** TypeScript is more verbose than JavaScript. Nobody adopted it to
type less; they adopted it for autocomplete, safe rename, and knowing what breaks before running. Same here.
You write more so the tooling knows more.

Never justify a design decision by counting characters. If a shorter form and a longer form compete, the
longer one wins unless human reading genuinely suffers.

**It is not a replacement for LaTeX and it does not compete with Typst.** Typst asks an author to abandon
their corpus, their journal templates, and their coauthors. ExactTeX asks them to keep all three. These are
different strategies serving different people. Compare tools by what they demand of the user, never by
overlapping feature lists.

## 3. What actually carries the weight

A novelty check on 2026-08-28 found that three things this project might have claimed are already built:

| Already exists | By whom |
|---|---|
| A document language with typed entities that emits LaTeX | **MyST** (active) |
| Semantic annotation layered on a LaTeX superset, with IDE validation | **sTeX / sTeXIDE** (2010) |
| A language server over LaTeX with completion and navigation | **texlab**, **digestif** |

Two things were not found anywhere, and they are the reason this project exists.

### 3.1 Errors stated in the author's own words

LaTeX today:

```
Overfull \hbox (12.3pt too wide) in paragraph at lines 45--47
```

ExactTeX:

```
your table "results" runs 12.3pt past the right margin
paper.xtex:212 — column 3 does not fit the width you declared
```

Same fact, said in the author's terms, using the name the author gave the object. This is only possible
because the author declared the entity. **That is what the syntax is for: it gives the tooling names to speak
with, not brevity.**

An agent repairing a document needs this even more than a human does. A human annoyed by a misplaced error
complains; an agent edits the wrong place and loops without converging.

### 3.2 A change model inside the file

Word stores tracked changes inside the `.docx`. That is why AI tools can propose edits a human accepts or
rejects — the data structure is already in the format.

LaTeX has nothing equivalent. So every tool builds its own layer around the file, and none of them
interoperate. ExactTeX puts the model in the format: any tool — an editor, an agent, a review UI — reads the
same anchored changes.

Consequence that matters more than the Word comparison: **an agent emits suggestions instead of edits.** The
human accepts or rejects. That is the supervision layer for machine-written text.

### 3.3 The enabler: faithful transport

An existing `.tex` passes through byte for byte. Untouched.

This is what separates ExactTeX from converters. MyST *converts* LaTeX into its own format; its own
documentation calls that "a transitional solution" and states it is not a full LaTeX renderer. Converting is
a one-way trip. Transporting lets you go back, hand the file to a coauthor, and submit it.

Transport is the on-ramp that makes 3.1 and 3.2 reachable from documents that already exist, not the
product in itself.

## 4. Binding design rules

1. **Explicit marker over short form.** `@ref(runtime)`, not `@runtime`.
2. **No important semantic distinction encoded in a single character.** LaTeX is full of these traps —
   `\citep` vs `\citet`, `$…$` vs `$$…$$`, `\ref` vs `\eqref`. A model sampling tokens gets a one-character
   distinction wrong, and the mistake is invisible in a diff. Use named fields: `style=textual`.
3. **Locality.** A construct's meaning depends only on nearby context. For the parser this is simplicity; for
   an agent it is a context-window property — writing section 4 must not require the whole document.
4. **An entry token must not be able to appear in ordinary LaTeX prose.** Otherwise renaming a `.tex` can
   silently change what it means. `figure runtime {` fails this: it is already valid LaTeX that typesets as
   text.
5. **Optimize writing for the agent, reading for the human.** The agent writes the bulk; the human reads it.
6. **A new construct enters only when it cannot be expressed with existing ones.** Inherited from the v0.4
   design, and it still holds. `@id(x)` hanging off any LaTeX construct already covers theorems, algorithms
   and anything else — those do not need their own block until typed checks are wanted.

## 5. Binding correctness properties

These are the gradual guarantee (Siek, Vitousek, Cimini, Boyland, SNAPL 2015) instantiated for documents.
**They cover two different regions of the file, and confusing them destroys the syntax.**

**A — Transport.** For input `u` containing no ExactTeX constructs:

```
emit(parse(u)).tex == u        byte equality
check(parse(u)) has no errors
```

**B — Typesetting equivalence.** For any document `d` that passes checking:

```
render(tex(emit(d).tex)) == render(tex(emit(erase(d)).tex))
```

Annotating must never change a pixel of the PDF, and must never turn a passing build into a failing one.
Test it by fuzzing: add valid annotations at random positions, rebuild, compare rasters.

**Erasure, never injection.** ExactTeX emits no assertions, wrapper environments, or support packages into the
output. Injection violates property B and collides with packages and catcodes.

## 6. Binding checking policy

A hard error — non-zero exit — is produced **only inside explicit ExactTeX constructs**.

Ordinary LaTeX is `?O`, the unknown *open* datatype: any package may define new constructors, so its type is
not merely unknown, it is unbounded. `?O` is consistent with every type. Therefore an unannotated `\ref{x}`
can never produce a hard error, and a renamed `.tex` checks clean by construction.

Everything else is `advisory` and never affects the exit code. Which advisories are printed depends on who
asked. An observation about plain LaTeX that nobody asked us to check is printed only behind `--strict-tex`.
A check an explicit construct *did* ask for, and that we could not perform, is printed by default — staying
quiet there reports a document as checked when it was not.

Never scan inside opaque regions for `\label`, `\ref` or `\cite` and treat what you find as checkable. Such a
scan matches inside a `\newcommand` body, inside verbatim text, and inside an inactive `\if` branch.
Advisory only.

`xtex check` reports **coverage**: what fraction of the document is checked versus raw LaTeX. This is the
analogue of `any` and `noImplicitAny`. It is the signal for supervising an agent-written draft.

## 7. Binding language for claims

The admissible phrasing is **"this combination is not assembled"**. Never "we are the first", "the only", or
"novel".

Do not describe ExactTeX as "a typed document language that compiles to LaTeX", or claim static reference and
citation checking as the contribution. Those are occupied. Lead with 3.1 and 3.2.

Any priority or superiority claim requires a fresh novelty check against live sources before it ships.

## 8. No anthropomorphism

Describe the compiler, the checker and any model structurally. It locates defects, resolves symbols,
transports bytes, and reports diagnostics. It does not understand, know, see, think, or believe.
