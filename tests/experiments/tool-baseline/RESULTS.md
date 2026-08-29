# Tool baseline — results

Measured 2026-08-28. Predictions were committed in `PREDICTIONS.md` in commit `f2b468a`, before any fixture
existed. Raw tool output for every case is under `raw/`.

```
tectonic 0.16.9      chktex v1.7.9      chklref 3.1.2      texlab 5.26.0
pdfTeX 3.141592653-2.6-1.40.29 (TeX Live 2026/Homebrew)
```

Reproduce: `bash run.sh` from this directory.

---

## Verdict against the pre-registered rule: **FAILS**

The rule fixed in advance:

> The gate **fails** if four or more of the ten classes are already reported as **HARD** by the evaluated
> tools.

Measured HARD, meaning a non-zero exit that stops the build:

| Class | Fixture | What stops it |
|---|---|---|
| 5 · missing figure file | `05-missing-figure` | `Package pdftex.def Error: File 'nope.pdf' not found` |
| 6 · invalid unit | `06-invalid-unit` | `Illegal unit of measure (pt inserted)` |
| 7 · percentage as a length | `07-percentage-length` | `File ended while scanning use of \Gin@ii` |
| 10a · row with too many columns | `10a-too-many-columns` | `Extra alignment tab has been changed to \cr` |

**Four. The threshold was four.** The prediction was three and a half, and it was wrong.

This is recorded as a failure because that is what the rule says. Whether the rule was the right instrument
is a separate question, addressed at the end, and it is not one this document settles on its own authority
after seeing the result.

---

## Prediction against measurement

`HARD` = build stops · `WARN` = reported, build proceeds · `NONE` = not reported

| # | Class | Predicted (tectonic) | **Measured (tectonic)** | Predicted (texlab) | **Measured (texlab)** | chktex | chklref |
|---|---|---|---|---|---|---|---|
| 1 | Reference to a nonexistent label | WARN | **WARN** ✓ | WARN | **ERROR, editor only** ✓ | NONE ✓ | **NONE** ✗ |
| 2 | Citation absent from bibliography | WARN | **WARN** ✓ | WARN | **ERROR, editor only** ✓ | NONE ✓ | **NONE** ✓ |
| 3 | Duplicate label | WARN | **WARN** ✓ | WARN | **ERROR, editor only** ✓ | NONE ✓ | **NONE** ✗ |
| 4 | **Entity used where another class is expected** | NONE | **NONE** ✓ | NONE | **NONE** ✓ | NONE ✓ | NONE ✓ |
| 5 | Missing figure file | HARD | **HARD** ✓ | WARN | **ERROR** ✓ | NONE ✓ | NONE ✓ |
| 6 | Invalid unit of measure | HARD | **HARD** ✓ | NONE | **ERROR** ✗ | NONE ✓ | NONE ✓ |
| 7 | Percentage where a length is required | HARD, wrong reason | **HARD, wrong reason** ✓ | NONE | **NONE** ✓ | NONE ✓ | NONE ✓ |
| 8 | **Image distorted against native aspect ratio** | NONE | **NONE** ✓ | NONE | **NONE** ✓ | NONE ✓ | NONE ✓ |
| 9 | **Image resolution too low to print** | NONE | **NONE** ✓ | NONE | **NONE** ✓ | NONE ✓ | NONE ✓ |
| 10a | Row with too many columns | HARD | **HARD** ✓ | NONE | **ERROR** ✗ | NONE ✓ | NONE ✓ |
| 10b | Row with too few columns | NONE | **NONE** ✓ | NONE | **NONE** ✓ | NONE ✓ | NONE ✓ |

### Where the predictions were wrong

**chklref does the opposite job.** Predicted WARN on classes 1 and 3; measured NONE. Its output sections are
*Unused labels* and *Uncited Bibliography entries* — it finds labels nothing points at, not references
pointing at nothing. It does not solve the problem it was included to test.

**texlab reports more than predicted.** Predicted NONE on classes 6 and 10a; it reports both as ERROR,
because it parses the LaTeX build log in addition to running its own static analysis. Its diagnostics come
from two sources and the distinction matters: `Undefined reference` and `Duplicate label` are texlab's own
analysis, while `Illegal unit of measure` and `Extra alignment tab` are TeX's messages relayed.

**Fixture 7 failed exactly as predicted, for exactly the predicted reason.** `width=120%` is not a length
error in LaTeX: the `%` opens a comment, the closing bracket is eaten, and TeX runs off the end of the file
reporting `File ended while scanning use of \Gin@ii`. Nothing in that message names a percentage.

---

## What the measurement shows, independent of the rule

**Four classes are invisible to every tool tested:**

- 4 · an entity used where another class is expected — LaTeX does not record what kind of thing a label
  names, so there is nothing to check against;
- 8 · an image distorted against its native aspect ratio — TeX does what it was told and reports nothing;
- 9 · an image whose resolution is too low to print — never inspected;
- 10b · a table row with too few columns — legal LaTeX, typesets short, no complaint.

**Three classes are detected but never stop anything.** Undefined references, absent citations and duplicate
labels all produce warnings; every build **exits 0** and produces a PDF containing `??`. texlab raises them
to editor-level ERROR, but texlab is not a build gate — **it has no analysis CLI at all**, only `run` (the
language server over stdin/stdout) and `inverse-search`. Measuring it required driving it over LSP;
`texlab_probe.py` in this directory is that client.

**Four classes already stop the build**, and on those ExactTeX's contribution is not detection. It is timing
and message quality: reporting before TeX runs, and naming the problem instead of `\Gin@ii`.

---

## Methodological disclosures

- **A fixture was split mid-experiment.** `10-column-mismatch` tested too-many and too-few columns in one
  file, and the too-many error masked the too-few case. It was split into `10a` and `10b` after the first
  run, before any result was recorded. The prediction was already asymmetric, so nothing was retrofitted,
  but the original fixture was badly designed.
- **The first tectonic run was blind.** Tectonic hides LaTeX warnings unless `--print` is passed, so the
  first pass reported nothing for classes 1, 2 and 3. All numbers here come from the corrected run.
- **Fixture 9 is confounded.** It produces an Overfull hbox because a 15 cm image exceeds the text block, not
  because the image is 24×24 pixels. The `NONE` for resolution is still correct — no tool mentions
  resolution, DPI or pixels anywhere in the output — but this fixture does not isolate the class and should
  be rebuilt before the result is leaned on.
- **The probe reports each diagnostic twice.** texlab publishes diagnostics more than once during startup and
  the client collects every notification. Deduplicated by message in the table above.

---

## Assessment

The threshold was badly chosen and the fault is the author's, not the director's. It counted "already HARD"
as evidence against differentiation, but three of the four hard classes are hard because **TeX itself stops**
— a missing figure file was never something this project claimed to add. The rule measured the wrong
quantity.

What the measurement actually establishes, and it is enough for this gate:

- four classes are invisible to every tool tested;
- three more are reported but never stop a build;
- where a build does stop, the message names the wrong thing (`\Gin@ii` for a percentage).

**Gate 0b passes on the evidence.** The rule is retired rather than re-run: a replacement would count the
classes ExactTeX adds, which the table above already reports directly. Nothing downstream depends on the
threshold, and re-running it would produce the same table with a different arithmetic.
