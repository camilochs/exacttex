# The transport corpus

Real LaTeX, used to decide how the shallow parser behaves. Two halves with different rules.

| | Where it lives | Why |
|---|---|---|
| **Real documents** | referenced, never copied here | licences we do not hold |
| **Adversarial fixtures** | in this repository | ours, and written to break things |

---

## Real documents are referenced, not vendored

`manifest.toml` records, per file: where it came from, under what licence, and a SHA-256 of the exact bytes
measured. It does not contain the file.

The reason is ordinary caution. A journal submission's copyright frequently sits with the publisher, and
this repository is MIT. Recording a fingerprint claims nothing about the bytes except that they are the ones
the number was computed from — which is the only claim a measurement needs.

It is also better evidence. Anyone can point the tooling at their own corpus and get a comparable number,
and the fingerprint makes a stale result detectable rather than silently reused. That is the measurement
rule in [`AGENTS.md`](../../AGENTS.md) §7 applied to our own inputs.

```sh
python3 tests/corpus/measure.py ~/Workspace          # measure a corpus
python3 tests/corpus/measure.py --verify manifest.toml   # check the fingerprints still match
```

---

## The thresholds, recorded before the measurement

The parser has three confidence levels, and the one that matters is `OpaqueToEof`: once a file enters it,
nothing further in that file is recognised. A file quarantined early is a file an author cannot annotate.

**The metric.** For each file, the byte position where `OpaqueToEof` begins, as a fraction of the file's
length. A file that never quarantines scores `1.0`. A file quarantined at its first byte scores `0.0`.

**What would count as failure**, fixed on 2026-08-29, before any corpus was measured:

| | Threshold |
|---|---|
| Median file | **≥ 0.90** available before quarantine |
| Files quarantined before half their bytes | **≤ 10%** |

The first is the one that matters. ExactTeX's on-ramp is "rename a `.tex` and start annotating", and an
author whose typical document goes dark at 60% cannot do that — the promise fails for the median case, not
the worst one.

The second catches a different shape: a small number of files that quarantine almost immediately would not
move the median but would mean a whole class of documents is unusable, and that class needs naming rather
than averaging away.

**Why these numbers and not others.** They are a judgement, not a derivation, and stating them before
measuring is what stops them from becoming whatever the first result happened to be. Missing them is not
proof the parser is wrong; it is a signal to look at *which* files failed and why, and either fix the
handling or record the class as unsupported with its reason.

---

## Adversarial fixtures

Under `hazards/`, one directory per parser hazard in [`ROADMAP.md`](../../ROADMAP.md)'s table. These are
written here rather than found, because a corpus contains what its authors happened to write and the
dangerous cases are exactly the ones nobody writes on purpose.

Each carries the observation that would show its handling is wrong — the roadmap's own falsifier — so a
fixture that starts passing for the wrong reason is visible.

---

## Baseline, 2026-08-29 — and why it does not mean what it says

```
555 files
  median available before quarantine   1.000   (threshold >= 0.900)
  quarantined before half their bytes  0.0%     (threshold <= 10%)
  never quarantined                    555
thresholds: met
```

**Do not read this as the parser passing.** It is the measurement taken *before* the shallow LaTeX parser
exists, and what it records is that almost nothing quarantines yet — because almost nothing is detected yet.

The hazard fixtures say so directly. Three of them are specified to enter `OpaqueToEof`, and on this build
none of them does:

| Fixture | Specified | Today |
|---|---|---|
| `01-verb-without-a-terminator` | `OpaqueToEof` | none |
| `03-catcode-at-top-level` | `OpaqueToEof` | none |
| `04-makeatletter-unmatched` | `OpaqueToEof` | none |

A number that reports success while measuring the absence of the thing it names is the failure mode
[`AGENTS.md`](../../AGENTS.md) §7 exists to catch, so it is written down here rather than quoted as a
result.

**What the baseline is good for** is the comparison. When [#21](https://github.com/camilochs/exacttex/issues/21)
lands, quarantine will start firing, and the same command on the same fingerprinted bytes says whether it
fires where it should or everywhere. A threshold met before the parser existed and missed after it would be
information; a threshold met in both cases would mean the parser changed nothing.

Re-run it against this manifest rather than against a fresh sweep, or the comparison is between two
different corpora.

---

## With the parser, 2026-08-29

```
327 files
  median available before quarantine   1.000   (threshold >= 0.900)
  quarantined before half their bytes  1.5%     (threshold <= 10%)
  never quarantined                    321
thresholds: met
```

**Every file in that 1.5% is one of our own hazard fixtures**, written to quarantine. Of the author's
documents, none goes dark early.

That is the number to compare against the baseline, and it is a different number from the first run of this
same command. Two things changed between them, and only one is a fix.

### The corpus definition was wrong, and it was changed after seeing the result

The first run swept `.tectonic-cache/bundles/`, which holds all of TeX Live — `pgf`, `tikz`, `expl3`. Those
are macro implementations full of `\catcode` and `\makeatletter`, and quarantining them is correct rather
than a failure. Counting them measured the distribution instead of the author's writing.

Excluding build caches was the right corpus definition and should have been written before measuring.
Changing it afterwards is recorded here rather than quietly applied, because "the threshold failed so I
narrowed the corpus" and "the corpus was wrong" look identical in a diff and are not the same thing. The
test that distinguishes them: a build cache is not the author's writing whatever the number had said.

The thresholds themselves were not moved.

### The parser had a defect, and the corpus is what found it

With caches excluded the rate was still 15.9%, and the files were real papers going dark at 6% of their
bytes. One byte was responsible:

```latex
{\LARGE\bfseries CERTAIN}\\[4pt]
                          ^^
```

`\\[4pt]` is a line break with its optional argument. The `\[` inside it was being read as display-math,
which then scanned for a `\]` that never came, and quarantined the rest of the document. **448 occurrences
across 71 files**, each one taking its document with it.

The parity rule that fixes it already existed for backslash runs; display-math opening simply was not
using it.

That is the corpus doing the job it was assembled for. No hand-picked example would have surfaced it,
because nobody writes `\\[4pt]` to test a parser — they write it to space a title block.


---

## Evidence from someone else's LaTeX

The corpus above is one author's. Four permissively licensed public projects were added to get a reading
that does not depend on this repository's author:

| Project | Licence |
|---|---|
| `vdumoulin/conv_arithmetic` | MIT |
| `soulmachine/leetcode` | BSD-3-Clause |
| `HarisIqbal88/PlotNeuralNet` | MIT |
| `sb2nov/resume` | MIT |

```
30 files
  median available before quarantine   1.000
  quarantined before half their bytes  0.0%
  never quarantined                    30
```

And all 30 check clean unrenamed, which is the on-ramp promise holding on documents nobody here wrote.

They are cloned rather than vendored, like everything else here — `git clone --depth 1` and point the
tooling at the directory. Their licences would permit vendoring; the fingerprint discipline applies anyway,
because a measurement whose input cannot be identified is not reusable.

One thing this does **not** establish is property B. Every eligible caption in those four uses
`\caption{\label{x} Text}`, which `tests/render/README.md` records as not migratable, so the property suite
skipped all of them. Transport and checking are confirmed on other people's LaTeX; annotating it is not.
