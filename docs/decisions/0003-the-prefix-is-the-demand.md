# 0003 · The identifier prefix is the reference's demand

**Status:** accepted, 2026-08-29. Decided by the director.
**Issue:** [#11](https://github.com/camilochs/nexttex/issues/11)

---

## The gap this closes

A type system needs two sides. NextTeX had one.

`\figure(fig:main)` declares a `Figure` — the class is known the moment the scanner reads the entry token.
But `@ref(fig:main)` demands nothing, so there is nothing a declaration can contradict, so nothing can be
checked. The class was computed and discarded.

In TypeScript the demand is written at the use site:

```ts
function f(x: number) { … }
f("a")                       // error: the signature says what it wants
```

The equivalent question here is how `@ref` says what it wants.

## The decision

**The prefix before the first `:` is the demand.** `@ref(fig:main)` demands a `Figure`. Pointing it at a
`\table(fig:main)` is `NT1004`.

Three options were on the table. This one was chosen because it costs nothing in the corpus it has to work
on, and because the syntax was already frozen — it adds no new form.

| Option | |
|---|---|
| **The prefix is the demand** | chosen |
| `@ref:figure(x)` states it explicitly | rejected: more to write, and it changes frozen syntax |
| No demand; the class only powers hover and rename | rejected: leaves the check that motivated the type system unbuilt |

## What made it free

Every `\label` in the author's corpus, 1,374 of them:

| Prefix | Count | Class |
|---|---|---|
| `sec` | 463 | Section |
| `fig` | 413 | Figure |
| `tab` | 277 | Table |
| `app` | 101 | Appendix |
| *(no prefix)* | 30 | — |
| `alg` | 25 | Algorithm |
| `subsec` | 24 | Section |
| `appendix` | 12 | Appendix |
| `ssec` | 11 | Section |
| `cap`, `eq` | 4 each | Section, Equation |
| `def` | 3 | `?O` — no admitted class |
| `chap`, `lst` | 2 each | Section, `?O` |
| `subsubsec`, `algo` | 1 each | Section, Algorithm |

**Only 30 of 1,374 carry no prefix at all**, and within a typed environment the prefix matches the
environment essentially always: `fig:` on 413 of 417 labelled figures, `tab:` on 272 of 276 tables, `alg:` on
25 of 26 algorithms.

The convention is already there. This decision reads it rather than imposing it.

## The default map, and why it is configurable

```toml
[prefixes]
figure    = ["fig"]
table     = ["tab"]
section   = ["sec", "subsec", "subsubsec", "ssec", "chap", "cap"]
appendix  = ["app", "appendix"]
algorithm = ["alg", "algo"]
equation  = ["eq"]
```

The corpus is why `section` has six spellings and `appendix` two. One author writes `subsec:`, `ssec:` and
`sec:` for the same class in one project. A map with a single spelling per class would report a type error on
54 correct labels in this corpus alone, which is the failure mode this whole document exists to avoid.

`nextex.toml` replaces the map entirely, never merges into it. An author whose convention is `figura:` writes
their own and nothing above applies.

## Where it does not fire

**An unmapped prefix demands nothing.** `def:geakg` and `lst:algorithm` are not in the map, so the reference
is `?O` and cannot fail. Adding a prefix to the map is how you opt a class into checking, and not adding one
is a valid permanent state.

**No prefix demands nothing.** The 28 unprefixed labels keep working exactly as they do today.

**A target of unknown class cannot fail.** `@ref(fig:x)` pointing at an `@id` attached to a `\newtheorem`
NextTeX does not model is `Known(Figure) ~ ?O`, which is consistent. Both sides must be known, per
[`checking.md`](../checking.md) §3.

This is the same renunciation TypeScript makes with `any`, and in a document most entities are `any`.

## What it catches

The failure that motivated it, and that no LaTeX tool reports:

```latex
\table(fig:results) { … }          % renamed from a figure, label kept
As shown in Figure~\Cref{fig:results}
```

LaTeX compiles it. `\Cref` reads the counter and prints "Table 4". The prose says "Figure". The PDF is wrong
and silent. The author uses `\Cref` 110 times across 4 projects, so this is a defect they can actually hit.

## What would reverse it

A corpus where the prefix convention is weak — say under 90% agreement between prefix and environment. Then
the map produces false errors and the explicit form becomes the better trade. Re-run
`tests/experiments/label-prefixes/` on that corpus before arguing it.
