# Checking

What the compiler is willing to call an error, and what it refuses to.

The rule underneath everything here is one line: **a diagnostic is a claim, and a claim needs evidence.** A
check that cannot substantiate itself stays quiet. That is not politeness — a checker that guesses trains its
author to ignore it, and an ignored checker has no value at all.

This document is the contract the checker, the CLI, the LSP and the test suites all read. The binding rules
it obeys are in [`PHILOSOPHY.md`](../PHILOSOPHY.md) §5 and §6 and in [`AGENTS.md`](../AGENTS.md) §4; they are
referenced here rather than restated.

---

## 1 · Only explicit constructs are checked

`@ref`, `@cite`, `@id` and the typed blocks carry the guarantee. The author wrote them, so the compiler
knows what they mean and may fail on them.

Plain LaTeX does not. A `\cite{x}` in transported bytes may come from a macro body, an inactive branch of a
conditional, or a package that redefines it. The compiler has no way to tell, so it says nothing.

Concretely: `@cite(invented2026)` is checked. `\cite{invented2026}` on the line below it is not.

This is what makes renaming a `.tex` to `.ntex` and changing nothing check clean. It follows by
construction, not from care taken case by case.

---

## 2 · Entity classes and the unknown open type

Every checked thing has a class:

```
Text  Block  Figure  Table  Equation  Section  Citation  Length
```

Everything else — every unknown control sequence, every environment the compiler does not model, every
region it quarantined — is `?O`: the unknown **open** datatype.

The word *open* is doing work. `?` in a gradual type system means "unknown among a fixed set of types". LaTeX
has no fixed set: any package may define new constructors at any time, so the unknown here is unbounded.
The term is from Malewski, Greenberg and Tanter, *Gradually Structured Data* (OOPSLA 2021).

Comparison is **consistency**, not equality:

```
Known(A) ~ Known(B)   if and only if A == B
?O       ~ T          for every T
T        ~ ?O         for every T
```

The second and third lines are the whole checking policy in two symbols. `?O` is consistent with
everything, so nothing involving unmodelled LaTeX can ever be inconsistent, so nothing involving unmodelled
LaTeX can ever fail. `?O` does not mean *invalid*. It means *the compiler has no grounds*.

---

## 3 · When the compiler may fail

A **hard error** sets a non-zero exit code. The list is closed: if a condition is not on it, it is not a hard
error.

| Code | Condition | Class involved |
|---|---|---|
| `NT1001` | Two `@id` constructs declare the same identifier in one document root | any |
| `NT1002` | An identifier is empty or contains bytes an identifier may not | any |
| `NT1003` | `@ref(x)` where no `@id` in the root declares `x` | any |
| `NT1004` | `@ref(x)` demanding class A on a target of known class B, A ≠ B | both known |
| `NT1005` | `@cite(k)` where `k` is absent from a bibliography read completely | `Citation` |
| `NT1006` | A `\figure` block whose image file does not resolve | `Figure` |
| `NT1007` | A length with an unsupported unit, or a percentage outside 0–100 | `Length` |
| `NT1008` | A block field that is required and absent, or present and malformed | `Figure`, `Table` |
| `NT1009` | An `@import` path that does not resolve | any |

Two properties hold across the whole table and are tested as properties, not as examples:

1. **Every row requires an explicit construct.** There is no row that ordinary LaTeX can reach.
2. **Every row requires both sides known.** `NT1004` cannot fire when either side is `?O`, and `NT1005`
   cannot fire when the bibliography is `Unavailable`. Uncertainty on either side means silence.

### Exit codes

| Code | Meaning |
|---|---|
| `0` | No hard errors. Advisories may have been printed. |
| `1` | At least one hard error from the table above. |
| `2` | Fatal: I/O failure, invalid annotation encoding, resource limit, broken internal invariant. |

Exit `2` is never reachable from unknown LaTeX. Unknown LaTeX downgrades confidence and is preserved; see
[`grammar.md`](grammar.md) §8.

---

## 4 · Advisory

An advisory names something the compiler noticed but cannot substantiate. It is:

- **off by default** — printed only behind `--strict-tex`;
- **never** able to change the exit code;
- marked `severity: advisory` in both output forms.

The class of things that are advisory and not errors:

- an unresolved `\ref` or `\cite` written in plain LaTeX;
- a `\label` in an opaque region that appears to collide with an `@id`;
- a bibliography that could not be read (see §7) — the advisory is about the file, never about a key;
- a region that entered quarantine early, which is a coverage signal rather than a defect.

**Never scan inside an opaque region and treat what you find as checkable.** Such a scan matches inside a
`\newcommand` body, inside verbatim text, and inside an inactive `\if` branch. This was a design error caught
in review, and it is the reason the rule is written as a prohibition rather than a preference.

---

## 5 · Coverage

`nextex check` reports what fraction of the document it checked.

```
coverage = 1 − (bytes in opaque nodes ÷ bytes in all nodes)
```

Byte-weighted, over the parsed document, computed by `Document::coverage`. An empty document is `1.0` by
convention: there is nothing unchecked in it.

**Coverage is a drop signal, not a threshold.** No number is a passing grade, and the compiler never fails on
one. What is worth acting on is a *fall*: a file that was 60% checked yesterday and is 30% today gained a
construct the parser cannot model, or entered quarantine early. Comparing a project against a fixed target
would only measure how much LaTeX that project happens to contain.

The one place an absolute figure means something: a fully annotated file that still reports low coverage is
reporting a parser gap, and that is what [issue #36](https://github.com/camilochs/nexttex/issues/36) is.

This is the analogue of TypeScript's `any` and `noImplicitAny`, and it is the signal for supervising a draft
an agent wrote.

---

## 6 · Erasure

`erase(d)` is `d` with every NextTeX construct replaced by the LaTeX it stands for and every opaque node
copied byte for byte.

Erasure emits **no** assertion, wrapper environment, or support package. This is binding
([`AGENTS.md`](../AGENTS.md) §4) and it is what makes property B testable at all: annotated and erased builds
are compared by rendering both and diffing the rasters. If emission injected anything, the two builds would
differ by construction and the property would be untestable rather than merely violated.

Practical consequence for anyone extending the emitter: a typed block lowers to the LaTeX its fields
describe and nothing else. A block that "helpfully" adds `\centering`, or loads a package its body needs, has
broken the contract even when the PDF happens to look right.

---

## 7 · Citations

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
stops. BibTeX accepts the file. Under the rule in §7, a parse failure is `Unavailable`, so adopting the
crate would silence citation checking for that document entirely — trading a dependency for lost coverage on
a file that works.

The key reader locates entry keys and nothing else. It does not read fields, resolve `@string` macros, or
validate an entry, because none of that is needed to answer the only question asked: does this key exist.
`@string`, `@comment` and `@preamble` declare no citation key and are skipped.

Reproduce it with [`tests/experiments/bib-parser`](../tests/experiments/bib-parser/). The evidence above is
a single run over one author's corpus, and it is what the decision rests on. A corpus where the crate parses
everything and the hand reader disagrees anywhere would reverse it.

---

## 8 · References

An `@ref` whose identifier no `@id` declares is `NT1003`. The scope is the document root — its root file
plus everything reached through `@import` — which matches LaTeX, where a label is document-wide.

Two `@id` constructs declaring the same identifier in one root is `NT1001`, blamed on the later one. The
first declaration is not at fault for existing.

`@cite` is excluded from this check even though it is also a reference: its keys come from a bibliography,
so it is answered by §7 instead. Answering it here would call an unread bibliography an absent key.

---

## 9 · Blame

Every diagnostic names which side of the compiler the offending bytes came from.

| Value | Meaning |
|---|---|
| `author-latex` | Bytes the author wrote as LaTeX and the compiler transported. |
| `nextex-construct` | Bytes the author wrote as NextTeX syntax. |
| `nextex-generated` | Bytes the emitter produced from a construct. |
| `unresolved` | No map segment supports an answer. |

`unresolved` is a real value and it is used. Guessing is worse than admitting the map does not reach: a
compiler that blames the author for its own generated bytes gets abandoned after the second time.

Blame matters most for errors NextTeX did not produce. When TeX fails, the source map converts its
`file:line` to an offset, finds the segment, and reports the origin — that is [issue
#14](https://github.com/camilochs/nexttex/issues/14) and it is why the map stores segments rather than
points.

---

## 10 · Diagnostics: two forms, one content

`nextex check` prints for a person. `nextex check --json` prints for a program. **Both carry the same
fields.** Neither form may hold something the other cannot express; a field added to one is added to both in
the same change.

The fields:

| Field | Meaning |
|---|---|
| `code` | `NT1001`…, stable across versions. |
| `severity` | `error` or `advisory`. |
| `blame` | One of the four values in §9. |
| `entity` | The class from §2, or `unknown-open`. |
| `name` | The identifier or key the diagnostic is about, when there is one. |
| `span` | File, byte offset, length, and the line/column derived from them. |
| `message` | One sentence, no trailing period, naming the thing rather than the rule. |
| `related` | Zero or more spans that explain the first, each with its own message. |

Human form:

```
error[NT1001]: identifier `fig:main` is already declared
  --> paper.ntex:88:14
   |
88 | \figure(fig:main) {
   |         ^^^^^^^^ declared again here
   |
  --> paper.ntex:41:9
   |
41 | @id(fig:main)
   |     -------- first declared here
   |
  blame: nextex-construct
```

JSON form, same diagnostic:

```json
{
  "code": "NT1001",
  "severity": "error",
  "blame": "nextex-construct",
  "entity": "figure",
  "name": "fig:main",
  "span": { "file": "paper.ntex", "offset": 2317, "length": 8, "line": 88, "column": 14 },
  "message": "identifier `fig:main` is already declared",
  "related": [
    {
      "span": { "file": "paper.ntex", "offset": 990, "length": 8, "line": 41, "column": 5 },
      "message": "first declared here"
    }
  ]
}
```

The LSP is a third rendering of the same record and adds nothing to it. If the LSP needs a field, it goes in
the table above first.

---

## 11 · What is never an error

- An unresolved `\ref` or `\cite` written in plain LaTeX.
- An unknown control sequence, environment, or package.
- Anything inside an excluded or quarantined region.
- Anything where one side of a comparison is `?O`.
- A low coverage figure.
- A bibliography that could not be read — the advisory is about the file.

The invariant this protects is in [`AGENTS.md`](../AGENTS.md) §4: renaming a `.tex` to `.ntex` and changing
nothing must check clean. It follows by construction from §1 and §3, not from care taken case by case.

The test that holds it is a property over the transport corpus, not a list of examples: for every file in
it, `check` on the renamed file must exit `0` and emit zero diagnostics of severity `error`.
