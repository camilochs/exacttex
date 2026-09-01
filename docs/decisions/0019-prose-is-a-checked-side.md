# 0019 · The word before a reference is a checked side

**Status:** accepted, 2026-09-01. Decided by the maintainer after Stage 0b's second measurement.

---

## The gap this closes

`decisions/0003` gave a reference a demand: the prefix before the first `:`. `@ref(fig:main)` demands a
figure, and pointing it at a `\table(fig:main)` is `XT1004`. That closed the case the motivating failure
was drawn from — a figure renamed into a table with its label kept.

It did not close the case the sentence around the reference makes. `Table~@ref(fig:main)` on a table is
correct by the prefix and wrong to a reader only if the prefix is what they read; `Figure~@ref(tab:main)`
on a table passes `XT1004`, compiles, and prints "Figure 3" above a table. Stage 0b
(`tests/experiments/0b-error-classes/`) measured this pair against tectonic, chktex, chklref, texlab and
`xtex check`, and none of the five reported it — `xtex` included, which the experiment's own results file
had first mis-described as caught. The prefix check reads the identifier; nothing read the prose.

## The decision

**The entity-kind word written immediately before a reference is a second demand, checked exactly as the
prefix is.** `Figure~@ref(tab:main)` on a declared table is `XT1020`:

```
error[XT1020]: prose says `Figure` but `tab:main` is a table
```

Both sides must be known, as for `XT1004`: the word must be one the vocabulary names, and the target's
class must come from a typed block or an `@id` the symbol table classifies. A `\label`, an `@id` on
unmodelled LaTeX, an absent word, a lower-case word or a word further than one separator away is silence.

## What "immediately before" means

The word ends at the construct's `@`, or one space before it, or one or more `~` before it — the three ways
a LaTeX author binds the word to the number. A line ending between them, a bracketed `\emph{Figure}`, or
"Figure 3 and @ref(…)" is not a binding, and the check does not guess.

The byte before the word is not a letter, so `Subfigure~@ref(…)` ends in a `figure` this does not read.

The vocabulary is fixed and small: the capitalised words, their plurals, and the abbreviations LaTeX
manuals use — `Figure`, `Figures`, `Fig.`, `Figs.`, `Table`, `Tables`, `Tab.`, `Section`, `Sections`,
`Sec.`, `Equation`, `Equations`, `Eq.`, `Algorithm`, `Algorithms`, `Alg.`, `Appendix`, `Appendices`. A
class the symbol table cannot assign — theorem, lemma, definition, listing — has no word, because a word
with no class to compare against could never fire and would only look like a check.

A `cref`-family reference generates its own word, so it is usually preceded by none, and then nothing is
checked. When an author writes one anyway — `Figure~@cref(tab:x)` — the word is read like any other, and a
list gets the word for every name in it: `Figures~@cref(tab:a, tab:b)` says both are figures.

## Why prose participates at all

Every other check reads what the author declared: an identifier, a block field, a bibliography key. Prose
is what the compiler otherwise transports without reading, and this record is the one place it reads a
word of it. Three things make that admissible here and nowhere else:

1. The word is bound to an explicit construct. The check fires only at a reference the author wrote as
   ExactTeX syntax; the same word before a plain `\ref{}` is transported and never read.
2. Both sides are declared. The target's class is the author's own declaration; the word is the author's
   own sentence. The compiler compares two things the author wrote and invents neither.
3. The defect is silent everywhere else. It compiles, it typesets a wrong word, and Stage 0b showed no tool
   reports it. A check that catches what nothing else catches is worth a fixed vocabulary of eighteen words.

## What was rejected

| Option | |
|---|---|
| **The word before the reference is a demand** | chosen |
| Read the word after the reference too (`@ref(x) shows the table`) | rejected: the binding is loose, and a false error on correct prose is worse than silence |
| A configurable vocabulary in `xtex.toml` | deferred: no measured corpus has asked for a word the fixed list lacks |
| Advisory rather than error | rejected: both sides are known, which is the `XT1004` condition for an error; an advisory here would say the compiler had grounds and declined to use them |

## What would reverse it

A measured corpus in which the fixed vocabulary produces false errors on correct prose — a field where
"Table" precedes references to things declared as something else — would move the check behind
configuration or to advisory. `tests/experiments/0b-error-classes/cases/prose-word/` holds the pair and
its clean control.
