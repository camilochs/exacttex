# 0006 · A typed block speaks placement

**Status:** accepted, 2026-08-30. Decided by the maintainer.
**Issue:** [#81](https://github.com/camilochs/exacttex/issues/81)

Phase 0a rewrote a published paper into the syntax and measured the gap that forced this: **634 of 662
figure and table environments in the author's corpus carry an explicit placement specifier (96%)** — `[t]`
alone appears 247 times. A typed block that cannot say `[ht]` cannot hold the corpus's normal case, so the
block form would go unused and the checked path with it.

## The chosen form

A `placement` field with a bare value:

```
\figure(fig:cd) {
  placement = !htbp
  ...
}
```

Zero new grammar: one more row in the §5 field tables, using the `bare` value kind that already existed.

## Rejected

- **Brackets after the identifier** — `\figure(fig:cd)[!htbp] { … }`. Reads familiar, but adds a grammar
  position to the block header, and the header is exactly what the §3 boundary rule keeps minimal.
- **A quoted string** — `placement = "!htbp"`. Quote noise for a value that never holds a space.

## Three rules that carry the semantics

1. Valid bytes are `h t b p !` (LaTeX's own) and `H` (the `float` package's), in any order. Anything else
   is a hard error naming the byte. Whether `float` is loaded is a package fact and stays unvalidated,
   like every other package fact.
2. The value is emitted verbatim inside `[ ]` — no reordering, no deduplication. The compiler does not
   improve author intent.
3. An empty field is malformed. Omitting the field is how no-brackets is asked for; writing the field and
   saying nothing is an error, not a synonym.
