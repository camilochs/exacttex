# Property B, measured

> Annotating must never change a rendered pixel, and must never turn a passing build into a failing one.

```
render(tex(emit(d)))  ==  render(tex(emit(erase(d))))
```

This is the property the on-ramp rests on. Until now it was a promise; this is where it becomes a
measurement.

```sh
python3 tests/render/compare.py <root> --binary target/debug/xtex
```

---

## The renderer noise floor is zero, and that was measured first

The comparison tolerance could not be chosen before knowing how much two identical builds differ on their
own. Three builds of the same document, compiled under different filenames:

| | Result |
|---|---|
| PDF bytes | **all three differ** — metadata and the document ID |
| Rasterised pages at 150 dpi, greyscale | **all three identical** |

So the tolerance is **zero**. Any pixel difference is a real difference, and the comparison is on rendered
pages rather than PDF bytes — because two builds that render the same ink already disagree about their
bytes.

Fixed settings, so the number means the same thing twice: `tectonic` for the engine, `pdftoppm -r 150 -png
-gray` for the rasteriser.

---

## What is annotated, and why that and nothing else

Real papers are already labelled, so there is nothing to *add* to them. What an author actually does on
their first day is convert:

```latex
\caption{Runtime}              \caption{Runtime}
\label{fig:runtime}      ->    @id(fig:runtime)
```

`@id(x)` emits `\label{x}`, so the built document should be the same document and the pages should be
identical. That is property B stated as the thing someone would really do.

**`\ref` is never touched.** `\ref` produces ink; a test that changed one would be measuring a real
difference and calling it a bug.

---

## Three things the suite got wrong before it got anything right

**It annotated inside a macro body.** The first run reported a real pixel difference on a real document, and
the compiler was innocent: the generator had inserted `@id(gen:1)` inside a `\newcommand` body, where
recognition is disabled. The annotation was transported literally and *printed*.

The fix is not a better pattern. **The compiler is the authority on where an annotation is eligible**, so
the harness now emits and checks: if `@id(` survives into the output, the position was not eligible and the
case is skipped rather than compared.

**It skipped every real paper.** A `.tex` copied alone into a temporary directory has no figures, no `.bib`
and often no class file, so nothing compiled and every document was skipped with a clean-looking report. The
suite now copies the whole project.

**It skipped every paper again, for the opposite reason.** The first generator only *added* labels where
none existed — and real papers have them everywhere. Migration is the right operation and it is also the
real one.

Each of those produced a run with zero failures. A suite that skips everything reports success exactly like
a suite that passes, which is the failure mode [`AGENTS.md`](../../AGENTS.md) §7 exists to catch, and it is
why the report prints its skip reasons rather than a total.

---

## Result, 2026-08-29

```
equal 14   failed 0   skipped 99
```

Fourteen real papers, each with 20-odd labels converted to `@id`, rendering pixel-identical. That is
property B confirmed on documents rather than on examples.

### A migration that is not safe, found by the suite

```latex
\caption{\label{fig:x} Computing the output values}
```

Moving that label out changes the output. `\label` prints nothing, but it is *present* — so the space after
it inside the caption is typeset. Move the label out and that space merges with the one `\caption` already
placed, and every caption's first word shifts by a space.

Nine pages of a real paper differed on exactly that, and the difference is one space per caption:

```
original   4.7:  The transpose of convolving a 3 × 3 kernel
migrated   4.7: The transpose of convolving a 3 × 3 kernel
```

The generator now attempts that shape only where no space follows the label. **This is a real limitation for
an author, not an artefact of the harness**: anyone migrating `\caption{\label{x} Text}` by hand will move
the same space.

### What the external corpus did and did not show

Four permissively licensed public projects — `conv_arithmetic` (MIT), `leetcode` (BSD-3), `PlotNeuralNet`
(MIT), `resume` (MIT) — were measured to get evidence from someone other than this repository's author.

| | Result |
|---|---|
| Transport and checking | **30 of 30 clean**, none quarantined |
| Property B | **no evidence** |

The second row is the honest one. Every eligible caption in those projects uses the shape above, which is
not migratable, so the suite skipped them all. They confirm the on-ramp on other people's LaTeX and say
nothing about annotating it.

---

## Skip reasons are part of the output

```
equal 42   failed 0   skipped 18
  skipped: the original does not compile here — 12
  skipped: nothing eligible to annotate — 5
  skipped: an annotation landed where recognition is disabled — 1
```

A document that does not compile on its own gives no baseline, so annotating it says nothing either way.
That is a limit of the corpus rather than a result, and it is printed so it cannot be mistaken for one.
