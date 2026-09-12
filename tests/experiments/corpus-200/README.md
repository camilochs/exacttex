# The second corpus: two hundred papers, stratified

The first corpus answered "does an unmodified LaTeX document survive ExactTeX
untouched, and what does the compiler find in published papers". It answered
with fifty papers from four computer-science categories, submitted from 2022 on.
Its document classes came out as it was sampled: `article` and `acmart` carry
three quarters of them, and `revtex`, `amsart` and `aastex` never appear.

Every claim about specificity is only as wide as the templates it crossed, and
the prevalence figure rests on fourteen cases, thirteen of them from a single
paper. This campaign widens the sample on the three axes that matter.

## The axes

| Axis | What it varies | Why |
|---|---|---|
| Field | computer science, maths, physics, statistics, biology, economics | different fields write different LaTeX |
| Year | 2014 to 2026 | idioms age; the first corpus starts in 2022 |
| Template | the document class the paper is written in | the parser meets a template, not a topic |
| Length | short papers and documents over a megabyte of source | the prevalence figure came from one long paper |
| Language | `babel` and CJK documents | encodings and hyphenation the first corpus barely met |

Quotas are targets, not promises. The licence gate has the last word: only
CC BY 4.0, CC BY-SA 4.0 and CC0 are redistributable, and how many papers carry
one of those differs field by field. Where a quota is not met, the shortfall is
in `harvest.log` and in the manifest, not hidden.

A paper that brings a document class the corpus lacks is taken even when its
field's quota is full, up to eight of that class. That exception is the campaign's
purpose written as a rule.

## What is inherited

The fifty papers of `../corpus/` are part of these two hundred, byte for byte,
marked `source=corpus-50` in the manifest. Every number this campaign produces
can be compared with the one it replaces, and the comparison is honest because
the old papers did not change.

The first corpus's rejections are carried too, so a paper already refused is not
asked about again.

## Running it

```sh
TARGET=200 python3 harvest.py          # resumable: rerun to continue
```

Politeness: at least three seconds between arXiv requests, an identifying
User-Agent, the licence read from the abs page before anything is downloaded.

## What each experiment runs on

| Experiment | Papers | Why not all two hundred |
|---|---|---|
| E1 specificity, E2 annotation ramp, E5 prefix census | 200 | cheap, and these are the claims that need the width |
| E3 planted defects vs incumbents | stratified 80 | recall is saturated; the cost is five twins per paper across four tools |
| E6 external verification | stratified 40 | the registries are asked politely, and two hundred bibliographies is both hours and a discourtesy |

Predictions for all of them were registered in `PREDICTION.md` before the first
new paper was downloaded.
