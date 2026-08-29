# 0001 · Typed emission, and no injection

**Status:** accepted, 2026-08-29. Decided by the director.
**Supersedes:** the `needs` field, and the README sentence promising package synthesis.
**Issue:** [#5](https://github.com/camilochs/exacttex/issues/5)

---

## The conflict this settles

Two documents in this repository prescribed different emitted bytes for the same typed block.

- `README.md` said a typed block "writes the packages it needs into the preamble for you".
- `AGENTS.md` §4 and `PHILOSOPHY.md` §5 say ExactTeX emits no support package, wrapper environment, or
  assertion of its own.

Both cannot be true. The emitter could not be written while they disagreed.

The director's original decision was that a block declares the packages it needs — a `needs` field. The
specification removed the field instead, and recorded that it was doing so against that decision. This record
closes the gap: **the decision is now to remove it**, taken by the director with the measurement below.

## The measurement that decided it

The proposal that would have kept the field was: `needs` is checked and never emitted. The block declares
what it requires, the compiler verifies the preamble already loads it, and nothing is written.

The question is whether that check can be trusted. Measured across 224 `.tex` files in the author's
workspace, grouped by project, for packages with a distinctive command:

| Package | Projects using it | With no explicit `\usepackage` |
|---|---|---|
| amsmath | 18 | 9 (50%) |
| hyperref | 35 | 16 (46%) |
| xcolor | 42 | 17 (40%) |
| graphicx | 30 | 10 (33%) |
| booktabs | 40 | 9 (22%) |
| subcaption | 16 | 3 (19%) |
| multirow | 6 | 1 (17%) |

Between a sixth and a half of real projects use a package's commands without ever writing its
`\usepackage`: a journal class or another package loads it. The compiler reads neither `.cls` nor `.sty`, so
it cannot see that. A `needs` check would have reported a missing package in 9 of the 40 projects that use
`booktabs` — each one a document that compiles.

A check that is wrong that often trains its reader to ignore it, and an ignored check has no value. That is
the argument in [`checking.md`](../checking.md), applied to the field that motivated it.

Reproduce with `tests/experiments/package-loading/`.

## The decision

1. **`needs` is not a field.** Writing it in a block is a malformed field, `XT1008`, like any other unknown
   key. Packages are declared where LaTeX declares them: `\usepackage` in the preamble, written by the
   author, transported byte for byte.
2. **A typed block lowers to the LaTeX its fields describe, and to nothing else.** The lowering is in the
   next section and it is exhaustive.
3. **`README.md` is corrected.** The sentence promising package synthesis is removed.

## What a typed block emits

`\figure(id) { … }` lowers to:

```latex
\begin{figure}
  \centering                                  % only if `width` or `src` is present
  \includegraphics[width=…]{src}
  \caption{caption}
  \label{id}
\end{figure}
```

`\table(id) { … }` lowers to:

```latex
\begin{table}
  \centering
  \caption{caption}
  body
  trailing
  \label{id}
\end{table}
```

The order is not a matter of taste. Measured across 662 float environments in the author's corpus:

| | figures (368) | tables (294) |
|---|---|---|
| `\centering` present | 97% | 88% |
| `\label` **after** `\caption` | 92% | 93% |
| `\caption` before the `tabular` | — | 96% |

`\label` after `\caption` is correctness rather than convention: `\caption` is what steps the counter, so a
`\label` placed before it records the previous number. The TeX FAQ states it directly — "if the label is
recording a `\caption` command, the `\label` command must appear after the `\caption` command, or be part of
it" ([FAQ-crossref](https://texfaq.org/FAQ-crossref)). Exactly one figure in 368 does it the other way.

Three rules govern every byte of that output.

- **Braced field content is copied, never reparsed or reformatted.** `caption`, `body` and `trailing` are
  spans into the source buffer. Whatever the author wrote inside them — comments, `\\`, nested braces,
  invalid UTF-8 — arrives unchanged. This is the transport invariant applied inside a construct.
- **Nothing is added that no field asked for.** No `\usepackage`. No wrapper environment. No `\FloatBarrier`,
  no `\vspace`, no package-specific helper, however much the output would benefit.
- **`\centering` is the one exception, and it is not injection.** It is part of what `\figure` and `\table`
  *mean* — the construct is a centred float, and an author who does not want one writes the environment in
  LaTeX. It is emitted unconditionally for `\table`, and for `\figure` only when there is an image to
  centre. Anyone extending the emitter should read this as the boundary rather than as licence: a second
  exception needs its own decision record.

## How the rule is tested

By comparing the two builds, which is exactly property B:

```
render(tex(emit(d))) == render(tex(emit(erase(d))))
```

`erase(d)` replaces every construct with the LaTeX it stands for and copies every opaque node. If the
emitter injected anything, the annotated build would carry bytes the erased build does not, and the rasters
would differ. So the property is not an extra check bolted onto the rule — the rule is what makes the
property testable at all.

The narrower form, testable before a TeX engine is wired in: emitting a document and emitting its erasure
must produce byte-identical `.tex` output outside the spans the constructs occupy.

## What this does not settle

Package synthesis is closed for v0.1, not forbidden for all time. Reopening it needs an end-to-end example
that requires a package absent from the source and still satisfies property B. None has been produced.

Reading `.cls` and `.sty` to see indirect loading is Phase 4 work. If it lands, the measurement above can be
re-run; a `needs` check would then be wrong far less often, and the case for the field could be made again
on new evidence rather than on the same evidence.
