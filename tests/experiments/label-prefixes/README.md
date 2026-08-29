# Can the identifier prefix carry the type demand?

Decides how `@ref` says what class it expects. If `fig:` reliably means "figure", the demand is already
written in every document and costs nothing. If it does not, the reference has to say so explicitly and the
frozen syntax has to change.

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

## What would reverse it

Agreement under about 90% between prefix and environment. Then the map produces false errors and the
explicit form (`@ref:figure(x)`) becomes the better trade.

## Limits

One author's corpus, and the strongest evidence in it comes from figures and tables. Equations are 3 labels;
that row proves nothing on its own. A second author's corpus with a different convention is the obvious next
measurement, and none was taken.
