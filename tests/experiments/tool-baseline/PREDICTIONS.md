# Tool baseline — predictions

**Written and committed before any tool was run.** This file exists so that the results cannot be
retrofitted into expectations. If a later commit edits a prediction in this file, the experiment is void.

Gate: repository issue #2. Question: for each error class ExactTeX proposes to catch before TeX runs, do
existing LaTeX tools already catch it, and how hard?

Tools, versions recorded at prediction time:

```
tectonic 0.16.9
ChkTeX   v1.7.9
chklref  3.1.2
texlab   (installing; version to be recorded in RESULTS.md)
pdfTeX   3.141592653-2.6-1.40.29 (TeX Live 2026/Homebrew)
```

Scale: **HARD** = non-zero exit or a stop; **WARN** = reported but the build proceeds; **NONE** = not
reported at all.

---

## Predictions

| # | Error class | tectonic | chktex | chklref | texlab |
|---|---|---|---|---|---|
| 1 | Reference to a nonexistent label | WARN | NONE | WARN | WARN |
| 2 | Citation absent from the bibliography | WARN | NONE | NONE | WARN |
| 3 | Duplicate label | WARN | NONE | WARN | WARN |
| 4 | **Entity used where another class is expected** | **NONE** | **NONE** | **NONE** | **NONE** |
| 5 | Missing figure file | **HARD** | NONE | NONE | WARN |
| 6 | Invalid unit of measure | **HARD** | NONE | NONE | NONE |
| 7 | Percentage where a length is required | HARD, for the wrong reason | NONE | NONE | NONE |
| 8 | **Image distorted against its native aspect ratio** | **NONE** | **NONE** | **NONE** | **NONE** |
| 9 | **Image resolution too low to print** | **NONE** | **NONE** | **NONE** | **NONE** |
| 10 | Table row with the wrong number of columns | HARD if too many, **NONE** if too few | NONE | NONE | NONE |

## Reasoning recorded with the predictions

**1, 2, 3 — soft everywhere.** LaTeX resolves references through the `.aux` file across two passes and
reports the failure as a warning, printing `??` in the output. The build succeeds. texlab has reported
undefined labels, undefined citations and unused BibTeX entries since v5.8.0 (July 2023), in the editor
rather than as a build gate.

**4 — nobody.** LaTeX has no notion of what kind of thing a label names. `\ref` yields a number; whether
that number belongs to a figure, a table or an equation is not represented anywhere. Nothing can check it,
because there is nothing to check against. This is the class ExactTeX adds rather than duplicates.

**5 — TeX itself, hard.** `\includegraphics` on a missing file stops the run. This class does *not*
differentiate ExactTeX; it only moves the report earlier and states it better.

**6 — TeX itself, hard.** `Illegal unit of measure` is a documented pdfTeX error.

**7 — hard, but for the wrong reason.** `width=120%` is not a length error in LaTeX: `%` opens a comment, so
the rest of the line disappears and the failure surfaces somewhere else entirely, or the document silently
typesets wrongly. Predicting HARD with the caveat that the diagnostic will not name the real problem. This
class is largely an artifact of ExactTeX's own syntax admitting percentages at all.

**8, 9 — nobody, and silently.** Setting both `width` and `height` against an image's native ratio distorts
it; TeX does exactly what it was told and reports nothing. Resolution is never inspected. Both fail at the
proof stage or not at all. These two are the strongest members of the class ExactTeX adds, and they are only
reachable because the block declares the dimension as a typed field.

**10 — asymmetric.** A row with too many `&` produces `Extra alignment tab has been changed to \cr`, which is
a hard error. A row with too few is legal LaTeX and typesets a short row with no complaint. So half of this
class is already covered and half is invisible. Note for our own checker: any column count must sum
`\multicolumn` widths, or it produces false positives on the 51 legitimate uses measured in the corpus.

---

## Decision rule, fixed before running

The gate **fails** if four or more of the ten classes are already reported as **HARD** by the evaluated
tools, because then the checking guarantee does not differentiate.

Counting the predictions above: HARD appears for classes 5, 6, 7 and half of 10 — **three and a half**. The
prediction is therefore that the gate passes, narrowly, and that the differentiation rests on classes 4, 8
and 9, plus turning the soft warnings of 1, 2 and 3 into a build gate.

If the measurement contradicts this, the measurement wins and the plan changes.
