# E1 — specificity, on two hundred papers

**Campaign 114613c, 2026-09-11.** Compiler `xtex-9731d47`, built from a `git archive` of that commit
(`../bin/README.md`). Every `.tex` file of every paper renamed to `.xtex` and nothing else changed,
then `xtex check --json` and `xtex build`, and the emitted file compared with the original byte
for byte. Produced by `run.py`; the per-file record is `out/files.csv`.

## The result

| | |
|---|---|
| Files | 647 |
| `check` exit 0 | 647 of 647 |
| Diagnostics | none: 0 errors, 0 advisories |
| Emitted byte-identical to the original | 647 of 647 |
| Bytes changed | 0 of 18,231,655 |
| Check time | 0.12 s at the slowest file |

Papers: 200, across 57 document classes and seven fields.

## What it answers

The prediction registered before the campaign (`../PREDICTION.md`, 3) was that at least 99% of files
would check clean and emit byte-identical output, and that any failure would come from a document
class the first corpus never met. There were no failures. Fifty-one document classes here were absent
from the first corpus — `revtex4-1`, `amsart`, `sigma`, `mnras`, `iopart`, `quantumarticle`,
`lipics-v2016`, `svjour3`, `aa`, `elsart`, `memoir`, among others — and each of them was carried
through untouched.

The claim the first corpus could make was "381 files of four computer-science categories". The claim
now is 647 files of seven fields, from 2009 to 2026, in fifty-seven templates. The gradual guarantee
holds where it was hardest to believe: the physics and astronomy classes, which redefine more of
LaTeX than anything in the first corpus.

## What it does not answer

Checking clean is the floor, not the ceiling: a file with no annotations has nothing to check, and
`?O` is consistent with everything. What this measures is that ExactTeX does not break a document it
was handed, and that renaming costs nothing. What the annotations buy is E2 and E5.
