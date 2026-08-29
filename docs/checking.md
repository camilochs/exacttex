# Checking

What the compiler is willing to call an error, and what it refuses to.

The rule underneath everything here is one line: **a diagnostic is a claim, and a claim needs evidence.** A
check that cannot substantiate itself stays quiet. That is not politeness — a checker that guesses trains its
author to ignore it, and an ignored checker has no value at all.

---

## 1 · Only explicit constructs are checked

`@ref`, `@cite`, `@id` and the typed blocks carry the guarantee. The author wrote them, so the compiler
knows what they mean and may fail on them.

Plain LaTeX does not. A `\cite{x}` in transported bytes may come from a macro body, an inactive branch of a
conditional, or a package that redefines it. The compiler has no way to tell, so it says nothing. This is
the same boundary the type system draws: an unknown control sequence is `?O`, the unknown *open* type, and
`?O` is consistent with everything.

Concretely: `@cite(invented2026)` is checked. `\cite{invented2026}` on the line below it is not.

---

## 2 · Citations

A `@cite` key is reported absent only when the bibliography behind it was read **completely**.

The bibliography is one of two things and never anything in between:

| State | Meaning |
|---|---|
| `Complete` | Every declared resource was found and every entry's boundary located. |
| `Unavailable` | Something failed. No key may be called missing. |

There is deliberately no partial state. A key set assembled from two of three `.bib` files looks complete
and is not: every key from the third file becomes a false "undefined citation" pointing at a line the author
wrote correctly. One unreadable file therefore silences citation checking for the whole document, and the
diagnostic that survives is about the file, not about the citation.

`Unavailable` carries why:

- **`NoneDeclared`** — the document declares no bibliography.
- **`ComputedPath`** — a declaration whose path is built by a macro rather than written literally. It cannot
  be resolved without running TeX, which the compiler does not do.
- **`Unreadable`** — a declared resource was not found.
- **`UnparsableEntry`** — an entry in a resource that was read has no locatable boundary.

### Where bibliographies are declared

Three forms, all found:

```latex
\bibliography{refs,extra}        % comma-separated, extension implied
\addbibresource{refs.bib}        % one path, extension written
\begin{thebibliography}{9}
  \bibitem{knuth1984} ...        % entries inside the document itself
  \bibitem[Knu84]{knuth1984b} ...
\end{thebibliography}
```

The third is not an edge case. Across 224 `.tex` files in the author's workspace, 14 files carry 501
`\bibitem` entries inline, against 39 `\bibliography{...}` declarations. A reader that handled only the
external form would report every key in those 14 files as missing.

Three details from the same sweep decide how the reader is written.

1. 18 of the 501 entries use the optional-label form `\bibitem[Knu84]{key}`, so skipping the label is
   required rather than defensive.
2. A further 10 occurrences of the word are `\verb+\bibitem+` inside prose about BibTeX, in 4 files that
   declare no bibliography at all. They are excluded before the reader runs, by the same `\verb` rule that
   governs every other construct — which is why the reader never sees them and those 4 files stay silent.
3. `\addbibresource` appears zero times. It is supported because `biblatex` documents use it, and that is a
   claim about the wider world, not about this corpus.

Declarations inside comments, verbatim, and math declare nothing — the scanner marks those regions excluded
before any of this runs.

### Why the key reader is written here

Issue #12 named the `biblatex` crate. It was measured against the hand-written key reader on 37 real `.bib`
files from the author's own projects:

| Outcome | Files |
|---|---|
| Identical key sets | 36 |
| `biblatex` failed to parse the file at all | 1 |

The one failure is `irace-package.bib`, shipped inside a widely used R package. It opens with an
`@preamble` that concatenates brace-delimited groups with `#`; the crate expects a quotation mark there and
stops. BibTeX accepts the file. Under the rule in §2, a parse failure is `Unavailable`, so adopting the
crate would silence citation checking for that document entirely — trading a dependency for lost coverage on
a file that works.

The key reader locates entry keys and nothing else. It does not read fields, resolve `@string` macros, or
validate an entry, because none of that is needed to answer the only question asked: does this key exist.
`@string`, `@comment` and `@preamble` declare no citation key and are skipped.

Reproduce it with [`tests/experiments/bib-parser`](../tests/experiments/bib-parser/). The evidence above is
a single run over one author's corpus, and it is what the decision rests on. A corpus where the crate parses
everything and the hand reader disagrees anywhere would reverse it.

---

## 3 · References

An `@ref` whose identifier no `@id` declares is an error. The scope is the document root — its root file
plus everything reached through `@import` — which matches LaTeX, where a label is document-wide.

Two `@id` constructs declaring the same identifier in one root is an error, blamed on the later one. The
first declaration is not at fault for existing.

`@cite` is excluded from this check even though it is also a reference: its keys come from a bibliography,
so it is answered by §2 instead. Answering it here would call an unread bibliography an absent key.

---

## 4 · What is never an error

- An unresolved `\ref` or `\cite` written in plain LaTeX.
- An unknown control sequence, environment, or package.
- Anything inside an excluded or quarantined region.
- Anything where one side of a comparison is `?O`.

These may become `advisory` diagnostics, which are off by default and never change the exit code.

The invariant this protects is in [`AGENTS.md`](../AGENTS.md) §4: renaming a `.tex` to `.ntex` and changing
nothing must check clean. It follows by construction from the list above, not from care taken case by case.
