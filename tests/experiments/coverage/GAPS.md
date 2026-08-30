# What the language cannot yet name

Source: a 70-line extract of arXiv:2603.27922 (lines 332–401 of its main file), SHA-256 of the
extracted chunk `49bec055…`. Following the corpus rule, the extract and its hand-annotated rewrite are
referenced by fingerprint, not stored — the findings below are what the exercise produced.

Chosen because it is the densest real mixture available: a definition environment, two tables, eight
references, eight citations, math inside table cells, and a table footnote.

---

## The gaps

### 1 · A table note has nowhere to live — and dropping it out breaks the table

The source ends the first table with a note attached to a `$^*$` marker in a cell:

```latex
\begin{table}
  ...tabular...
  \vspace{2pt}
  {\scriptsize $^*$Counts depend on the generated topology...}
\end{table}
```

The `\table(...)` block has fields for `caption` and `body` and nowhere for this. Writing it in a `latex { }`
escape moves it **outside** the emitted `\begin{table}…\end{table}`, which changes where it is typeset. So
the annotation as written in the rewrite is wrong, and the gap is not cosmetic: **the block cannot
express everything a real `table` environment contains.**

Options for Phase 1: a `note` field; a general "trailing content" field; or blocks that accept arbitrary
LaTeX in body position rather than only in named fields.

### 2 · The kind word is written by hand, every time

```
Definition~@ref(def:sixtuple)   Table~@ref(tab:kg_comparison)   Section~@ref(sec:results)
```

The author types `Definition~`, `Table~`, `Section~` before every reference. ExactTeX knows the kind — that
is the whole point of typed entities — so it could emit the word. It must not: generating it adds bytes that
were not there, which is injection, and injection is forbidden because it breaks the guarantee that
annotating never changes the output.

So the compiler knows something it is not allowed to use. Worth naming as a decision rather than leaving it
to surface later: either the word stays manual, or `@ref` gains a variant that generates it and the
no-injection rule gets an explicit, documented exception.

### 3 · `@` occurs inside tabular column specifications

```latex
\begin{tabular}{@{}lcc@{}}
```

The corpus count that justified the `@` entry token searched for `@` followed by a letter and did not see
this. It does **not** collide — the entry token requires `@` + keyword + `(`, and `@{` is neither — but the
grammar should say so explicitly instead of leaving a reader to work it out. Two occurrences in this
70-line chunk alone.

### 4 · `columns` duplicates information that is already in the source

`\begin{tabular}{@{}lcccccc@{}}` already states the column count. Declaring `columns = 7` restates it, which
means the two can disagree, and then the checker is validating an annotation against another annotation
rather than against the table. It should probably be read from the column spec instead of declared.

### 5 · Things with no way to be named at all

- **A table row.** Nothing can point at one.
- **A cell, and its footnote marker.** The `$^*$` in a cell and the note at the bottom are related; the
  language cannot say so.
- **The optional title of an environment** — `\begin{definition}[Generative Executable…]`. `@id` attaches
  after it and works, but the title itself is data the tooling cannot see.
- **An appendix as a kind.** `Appendix~@ref(app:symbolic_executor)` resolves to a section; nothing records
  that it is an appendix.

---

## Not measured here

**Size.** the extract is 70 lines and the rewrite is 52, but they are not comparable: item lines were
dropped from the definition to keep the hand-rewrite manageable. No size claim is made from this chunk. If
that number is wanted, it needs a chunk rewritten line-for-line with nothing omitted.

**Everything that was not in this chunk.** No algorithm environment, no subfigure, no `\textcolor` revision
block, no `generated/` file. Those were listed as expected gaps in issue #1 and remain unchecked. This chunk
covers definitions, tables, references and citations only.

---

## What goes into Phase 1

1. Decide how a block holds content that is not a named field (gap 1). This one blocks the table block.
2. Decide whether `@ref` may generate the kind word (gap 2).
3. State in the grammar why `@{` does not collide (gap 3).
4. Decide whether `columns` is declared or inferred (gap 4).
5. Decide which of the unnameable things get a way to be named, and which stay out (gap 5).
