# The campaign's data, and how to rebuild it

This directory is the research data behind the corpus experiments: the manifest
that names every paper, the log of every candidate refused and why, the
predictions registered before anything ran, the scripts, and the raw output each
figure in the paper was computed from.

## What is here, and what is not

The two hundred papers themselves are **not** committed. Their licences allow
redistribution — that is what the licence gate is for — but they weigh 1.4 GB,
and every one of them is a public arXiv entry that `manifest.csv` names by its
identifier. Rebuilding the corpus is one command:

```sh
python3 harvest.py          # reads manifest.csv, fetches what is missing
```

The harvester is polite by construction: at least three seconds between arXiv
requests, an identifying User-Agent, and a 503 with `Retry-After` honoured. It
is resumable; a run that is interrupted continues where it stopped.

## What each file answers

| Path | What it holds |
|---|---|
| `manifest.csv` | one row per paper: id, field, category, title, licence, document class, file counts |
| `rejections.csv` | every candidate refused, with the reason (licence, class cap, pdf-only, …) |
| `MANIFEST-114613c.md` | the corpus as frozen: counts by field, licence, document class and year |
| `PREDICTION.md` | what each experiment was predicted to say, registered before it ran |
| `README.md` | the sampling design: the axes, the quotas, and what is inherited from the first campaign |
| `harvest.log` | the harvest itself, line by line |
| `E1-specificity/` | rename-and-emit: `out/files.csv` is one row per source file |
| `E2-annotation-cost/` | the annotation ramp: `out/<id>.json` per paper, with the check's own answer |
| `E3-spike-in/` | planted defects: `out/results.csv`, and under `out/<id>/` the raw output of every tool on every twin |
| `E6-external-verification/` | external verification: the dated records, the per-paper diffs, and `out/author-classes.csv`, the rule-based reading of author-list differences |

## Provenance

Every number was produced by a pinned build of the compiler, built from a
`git archive` of its commit rather than from a working tree; the commit is named
in each experiment's results file. The first campaign's fifty papers are inside
these two hundred unchanged, so a figure here can be compared with the one it
replaces.
