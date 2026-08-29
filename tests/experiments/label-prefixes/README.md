# Which label prefixes does a real project actually use?

**This experiment does not decide whether the prefix convention exists.** That question is answered by
documentation, not by sampling: the [LaTeX2e reference manual](https://latexref.xyz/_005clabel.html) and the
[Wikibooks LaTeX book](https://en.wikibooks.org/wiki/LaTeX/Labels_and_Cross-referencing) both document it,
and between them they name `ch`, `sec`, `subsec`, `fig`, `tab`, `eq`, `lst`, `itm`, `alg` and `app`. That is
where the default map in `docs/decisions/0003` comes from.

What this measures is narrower and is the thing documentation cannot tell you: **which spellings a real
project adds beyond the documented set.** One corpus is enough for that, because the claim is an existence
proof — if a real project writes `ssec:`, then a fixed map would fail it, and the map has to be replaceable.

It is not enough for any claim about how common the convention is, and none is made here.

## Run

```sh
python3 measure.py ~/Workspace
```

## Result, 2026-08-29

1,374 labels across 224 `.tex` files.

| Prefix | Count |
|---|---|
| `sec` | 463 |
| `fig` | 413 |
| `tab` | 277 |
| `app` | 101 |
| *(none)* | 30 |
| `alg` | 25 |
| `subsec` | 24 |
| `appendix` | 12 |
| `ssec` | 11 |
| `cap`, `eq` | 4 each |
| `def` | 3 |
| `chap`, `lst` | 2 each |
| `subsubsec`, `algo` | 1 each |

Agreement between prefix and the environment the label sits in:

| Environment | Labelled | Dominant prefix |
|---|---|---|
| figure | 417 | `fig` — 413 (99%) |
| table | 276 | `tab` — 272 (99%) |
| algorithm | 26 | `alg` — 25 (96%) |
| equation | 3 | `eq` — 3 (100%) |

Only 30 of 1,374 labels carry no prefix, and inside a typed environment the prefix agrees essentially
always. The convention is already there to be read.

**The reason the map is configurable is also in this table.** One author writes `sec:`, `subsec:`, `ssec:`,
`subsubsec:`, `chap:` and `cap:` for the same class, and `app:` and `appendix:` for another. A map with one
spelling per class would report a type error on 54 correct labels in this corpus alone.

See [`docs/decisions/0003`](../../../docs/decisions/0003-the-prefix-is-the-demand.md).

## Limits, and they are the point

**One author's corpus.** These numbers are that author's habit. They are not a measurement of LaTeX
practice and must never be quoted as one — an earlier version of `decisions/0003` did exactly that and was
corrected.

Within the corpus, the strongest evidence comes from figures and tables. Equations are 3 labels; that row
proves nothing on its own.

## The measurement that has not been taken

Point this script at a broad sample of published LaTeX sources — arXiv distributes them — across areas and
authors. That would say how often an unmapped prefix is the rare case, which is the only thing that could
reverse the decision. Nobody has run it, and until someone does, "the convention is widespread" is not a
claim this repository makes.
