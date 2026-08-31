# 0018 · What a table overfull can honestly be blamed on

**Status:** accepted, 2026-08-31. The investigation issue #100 ordered
before any feature: establish against live engine runs what the log
actually localizes inside a `tabular`, whether declared widths give a
computable budget, and what the honest sentence is when the evidence
does not support column attribution.

Three runs against pdfTeX (TeX Live 2026, `textwidth=6cm` so everything
overflows on demand). Logs kept beside this record in spirit; the
shapes below are transcribed from them.

## What the engine localizes — it depends on the column type

**Paragraph-shaped columns (`p{…}`, `tabularx`'s `X`) localize to the
row and print the offending content.** Each cell is its own paragraph,
so an overfull inside one is an ordinary paragraph overfull with the
row's source line and — this is the decisive part — the box trace
carries the cell's own text:

    Overfull \hbox (137.35315pt too wide) in paragraph at lines 8--8
    []\OT1/cmr/m/n/10 ThisWordIsFarTooWideForItsColumn|

Line 8 is the row. The trace text appears in exactly one cell of that
row, and that cell's index **is** the column. No width arithmetic is
needed: the engine already named the content that did not fit, and
matching it against the row's cells is evidence, not inference.

**Natural-width columns (`l`, `c`, `r`) localize to nothing smaller
than the whole environment, and the trace is empty:**

    Overfull \hbox (93.0062pt too wide) in paragraph at lines 4--8
    [][]

Lines 4–8 are the entire `tabular`. This is not a reporting weakness to
work around — it reflects what actually happened. A natural-width cell
cannot fail to fit a width nobody declared; the TABLE as a whole is
wider than the line. "Column 3 does not fit" has no referent here.

## Do declared widths give a computable budget?

Yes — a `p{2.5cm}` is a number and the overfull amount is a number —
but the budget is **not needed for attribution**, because the trace
already carries the content. Computing "column 3's contents exceed its
2.5cm by 12.3pt" from our side would be recomputation of something the
engine states directly, and every recomputation is a place to diverge
from the evidence. The budget stays unused.

## The ambiguous case, measured

Two cells of the same row carrying the same too-wide word produce two
records with identical trace text and overlapping line ranges:

    Overfull \hbox (94.65866pt too wide) in paragraph at lines 11--11
    []|\OT1/cmr/m/n/10 SameWordTooWideForBoth|
    Overfull \hbox (94.65866pt too wide) in paragraph at lines 11--12
    []|\OT1/cmr/m/n/10 SameWordTooWideForBoth|

Content matching cannot tell these apart. The blame rule applies with
full force: **a confident wrong column is worse than a located table.**

## The sentences this licenses

| evidence | sentence |
|---|---|
| trace content matches exactly one cell of the named row | `table \`tab:results\` — column 3 does not fit; "ThisWordIsFar…" runs 137.4pt past the width you declared` |
| trace content matches several cells, or none | `table \`tab:results\` — row at line 8 runs 94.7pt past a declared width` |
| no trace content (natural-width columns) | `table \`tab:results\` runs 93.0pt past the right margin` — #101's sentence, already shipped |

Nothing below the evidence, nothing above it.

## The feature this scopes

Extend `texlog::parse_log` to keep the 1–2 trace lines that follow an
Overfull record (strip `[]`, `|` and font runs like `\OT1/cmr/m/n/10`);
in blame, when the mapped entity is a table, split the named row's
source line on `&` and match. One match → column named, content quoted.
Otherwise → the row/table sentences above. The `l/c/r` case needs no
new code: it is #101's sentence.
