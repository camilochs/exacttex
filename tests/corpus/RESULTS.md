# The external corpus — first measurement

Measured 2026-08-30, against the thresholds frozen in [`README.md`](README.md) before any corpus existed.
Corpus built by [`fetch.py`](fetch.py): **95 documents, 992 `.tex` files** nobody in this project wrote —
two papers per cell of an 8-year × 6-field grid (2005–2026; algebraic geometry, data structures, hep-th,
astronomy, statistical mechanics, population biology), plus five multi-file books and lecture-note
repositories pinned by commit. Provenance, licences and per-file SHA-256 in the corpus's
`provenance.json`; the measurement's own instrument (binary hash, commit) in its manifest.

## The verdict

```
992 files, 0 of them already annotated

  the two promises, over the 992 unannotated files
    check clean, unmodified              992/992
    emit the input's bytes exactly       992/992

  how much is reachable
    median available before quarantine   1.000   (threshold >= 0.900)
    quarantined before half their bytes  9.6%     (threshold <= 10%)
    never quarantined                    881

thresholds: met
```

**Both promises hold on every file of every year and field measured.** No unmodified document produces a
hard error, and every one emits its own bytes exactly. This is the first evidence for either claim that does
not come from this project's author's own writing.

The quarantine thresholds are met, but the second one barely — 9.6% against a ceiling of 10% — and the
margin is thin enough that the causes matter more than the pass.

## What stopped recognition, grouped

| Count | At the stopped byte | What it actually is |
|---|---|---|
| 102 | `\iftag` | OpenLogic's own conditional machinery — braced-argument conditionals across one book's files. Arguably correct quarantines of genuinely unscannable macros, but they are 92% of all quarantined files. |
| 2 | `\iff` | **The kernel's ⟺ symbol**, not a conditional. No `\fi` exists or is needed. |
| 2 | `\in` | **A half-open interval.** `X \in [A, B)` inside `\begin{equation}`: the `[` is read as an optional argument that never closes. |
| 1 each | `\ifabstract`, `\ifhyper`, `\ifdraft` | Real author conditionals opened in preambles — correct quarantines. |
| 1 | `\bigg` | Sized delimiter in display math, same family as `\in`. |
| 1 | `\makeatletter` | Correct. |

## The root cause behind the false quarantines

Reduced to minimal cases, verified in both directions:

```
\begin{equation} X \in [A, B) \end{equation}     → quarantine at \in
\begin{equation} X \in [A, B] \end{equation}     → none
$X \in [A, B)$                                    → none
\begin{align*} a &\iff b \end{align*}            → quarantine at \iff
```

Inline math is an exclusion region and behaves as one. **Display-math environments are promised as an
exclusion region by `grammar.md` §8 — "for a display-math environment, the first byte after the header
slot" — and the scanner implements no such set.** `equation`, `align`, and their family do not exist for
it, so their bytes are scanned as command syntax: `\iff` matches the `\if…` conditional rule, and an
unknown command followed by `[` opens an optional-argument scan that a half-open interval never closes.

Separately, `\iff` in prose — outside any math — also quarantines, so the conditional rule needs the kernel
symbol excepted regardless of the region fix.

Issues filed from this measurement: the display-math environment set, and the `\if…` rule's exceptions.

## Boundary

One run, 95 documents, two per cell — enough to find defect families, not to estimate their rates. The
per-cell sample is small and licence-filtered toward what arXiv exposes. The `\iftag` concentration shows a
single macro-heavy project can dominate the quarantine tally; conclusions about the *rate* of quarantine in
the wild need a corpus an order of magnitude larger, which belongs on arXiv's S3 bulk route rather than the
polite crawl `fetch.py` uses.
