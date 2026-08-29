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

## Where the default map comes from

**From the published convention, not from a corpus.** LaTeX has no specification for label names, but the
convention is documented, and two sources agree. Both transcribed on 2026-08-29:

| Source | Prefixes it names |
|---|---|
| [LaTeX2e unofficial reference manual](https://latexref.xyz/_005clabel.html) | `ch`, `sec`, `fig`, `tab`, `eq` |
| [Wikibooks, *LaTeX/Labels and Cross-referencing*](https://en.wikibooks.org/wiki/LaTeX/Labels_and_Cross-referencing) | those five, plus `subsec`, `lst`, `itm`, `alg`, `app` |

The reference manual: *"A common convention is to use key names consisting of a prefix and a suffix
separated by a colon or period."* Wikibooks: *"It is common practice among LaTeX users to add a few letters
to the label to describe what you are referencing"*, and *"You are not obligated to use these prefixes."*

So the default map is:

```toml
[prefixes]
figure    = ["fig"]
table     = ["tab"]
section   = ["sec", "subsec", "ch"]
appendix  = ["app"]
algorithm = ["alg"]
equation  = ["eq"]
```

`lst` and `itm` are documented but have no admitted entity class, so they stay unmapped and demand nothing.

## What the corpus is used for, and what it is not

One author's corpus cannot show that a convention is universal. It can show that something occurs, and that
is the only claim made from it here.

Measured across 1,374 labels in that corpus: the documented prefixes are used, and **so are six spellings
the documentation does not name** — `appendix`, `ssec`, `subsubsec`, `cap`, `algo`, `def`. Together they
account for 55 labels.

That is an existence proof and it settles one thing only: **the map has to be replaceable.** A fixed map
built from the published convention would report a type error on those 55 correct labels, in the first real
project it met.

It settles nothing about how common the convention is. `tests/experiments/label-prefixes/` says the same in
its limits section.

### An earlier version of this record over-claimed

It said the decision "costs nothing in the corpus it has to work on" and that the convention is "already
there", citing agreement rates from that one corpus as though they measured LaTeX practice. They measured
one author's habit. The decision stands, but it stands on the documented convention above and on the
fallback in the next section, not on that sample.

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

Evidence that the published convention is not followed widely enough for an unmapped prefix to be the rare
case — say under 90% agreement between prefix and environment across many authors. Then most references
would demand nothing, the check would rarely fire, and the explicit form would be worth its verbosity after
all.

That evidence does not exist yet in either direction. `tests/experiments/label-prefixes/` runs on any
corpus; pointing it at a broad sample of published sources is the measurement that would settle it, and it
has not been taken.
