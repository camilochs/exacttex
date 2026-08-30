# 0004 · What a percentage becomes, and what `@import` becomes

**Status:** accepted, 2026-08-29. Decided by the maintainer.
**Constraint he set:** resolve both without adding a keyword.
**Issue:** [#13](https://github.com/camilochs/exacttex/issues/13)

---

Three questions the emitter could not answer from the specification. None of the answers adds syntax: each
is a fixed rule attached to a field or a construct that already exists.

## 1 · `width = 80%` becomes `0.80\linewidth`

A percentage has to become a TeX length, and the reference changes the PDF. Compiled in a two-column
document rather than recalled:

| Inside | `\linewidth` | `\textwidth` | `\columnwidth` |
|---|---|---|---|
| `figure` — one column | **229.5pt** | 469.0pt | 229.5pt |
| `figure*` — spanning | **469.0pt** | 469.0pt | 229.5pt |

`\textwidth` overflows a single-column float. `\columnwidth` under-fills a spanning one. **`\linewidth` is
the only reference correct in both**, because it is the width of the line the float is actually set on.

Frequency in a corpus is the wrong metric here and was used at first: the author's papers write `\textwidth`
124 times, `\columnwidth` 65 and `\linewidth` 49, and that says nothing about which is correct. A rule the
compiler applies everywhere has to hold everywhere.

### Naming a different reference

The percentage is a shorthand for the common case and its reference is fixed. An author who needs another
one names it, and a `length` therefore admits a TeX length:

```
width = 80%                shorthand, 0.80\linewidth
width = 0.8\columnwidth     the reference named
width = \textwidth          no coefficient
width = 12cm                absolute
```

This was a hole rather than a trade. The first draft defined a `length` as a number plus one of six units,
which left `\columnwidth` unreachable without abandoning the typed block for plain LaTeX — and inside a
float spanning both columns, `\linewidth` is the full page width, so a column-width image there has no
percentage form at all. The restriction was written without anyone asking whether it should hold. The
maintainer asked.

A control word followed by `{` is a command taking an argument, not a length, and is rejected.

Checked before implementing: a backslash inside a field value does not disturb the block's brace counting.
The boundary is found at the same offset with and without one.

## 2 · `height = 40%` becomes `0.40\textheight`

`height` is admitted as a figure field, with the same value kinds as `width`.

There is no `\lineheight`. TeX exposes no "space available in this float", so no adaptive reference exists
the way `\linewidth` does for width. `\textheight` — the page's text height — is what the corpus uses
whenever a relative height appears, and it is the only length with a defensible meaning here.

The asymmetry with `width` is real and it is not a design choice. It is what TeX offers.

## 3 · `@import` becomes `\input`, and could not have become `\include`

Not a preference. LaTeX settles it:

```
error: middle.tex:1: LaTeX Error: \include cannot be nested.
```

`@import` nests by definition — [`grammar.md`](../grammar.md) §4 defines a document root as its root file
plus the **transitive closure** of files reached by `@import`. A construct that nests cannot lower to a
command that refuses to.

`\input` also imposes no page break, which `\include` does. Emitting a page break the author did not ask for
would be injection under [`decisions/0001`](0001-typed-emission-and-no-injection.md), so the two reasons
agree.

## What is still open: `keepaspectratio`

Giving `\includegraphics` both `width` and `height` stretches the image to fit unless `keepaspectratio` is
also given. In the measured corpus, of 243 `\includegraphics` calls:

| | Count |
|---|---|
| Give both `width` and `height` | 17 |
| …of those, also give `keepaspectratio` | **15** |

So an author who gives both almost always means a bounding box rather than a stretch. Emitting
`keepaspectratio` when a block gives both would follow that intent — and would be a **second exception to
no-injection**, which `decisions/0001` says needs its own record.

Not settled here. Until it is, a block giving both fields emits both and nothing else, and the image is
stretched exactly as plain LaTeX would stretch it.
