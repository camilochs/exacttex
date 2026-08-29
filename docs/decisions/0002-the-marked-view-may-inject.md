# 0002 · The marked view may inject

**Status:** accepted, 2026-08-29.
**Depends on:** [`0001`](0001-typed-emission-and-no-injection.md), which requires a second exception to
no-injection to carry its own record.
**Issue:** [#6](https://github.com/camilochs/exacttex/issues/6)

---

## The rule this bends

Decision 0001: a typed block lowers to the LaTeX its fields describe and nothing else. No support package,
no wrapper environment, no helper command, however much the output would benefit.

`xtex build --marked` renders every revision visibly — inserted text coloured, deleted text struck
through. There is no way to do that with kernel commands alone. It has to emit `\textcolor` and a
strikethrough command, and it has to load the packages that define them.

## Why it is permitted here

Because the property that no-injection protects does not apply to this view.

Property B says annotating must never change the rendered page. The whole purpose of `--marked` is to change
the rendered page: a reviewer opens it precisely to see what an ordinary build hides. Applying the property
here would be a category error, not a violation caught.

The rule and the property are not separable. No-injection exists *because* injection breaks property B and
collides with the author's packages and category codes. Where property B does not apply, the first reason is
gone; the second is handled by the bounds below.

## The bounds

1. **`--marked` output is never the artefact of record.** It is written to a distinct filename. A journal
   receives what `xtex build` produces, and that build is unchanged by this decision.
2. **The normal build is byte-identical whether or not `--marked` was ever run.** Testable directly, and it
   is the test that keeps this decision from leaking.
3. **The injected set is fixed and listed here**, not open to growth by whoever next needs something:
   `\usepackage{xcolor}` and `\usepackage[normalem]{ulem}`, plus `\textcolor` and `\sout` at each revision.
   Adding to that list is a change to this record.
4. **A collision is the author's to resolve, and it is visible.** If the document already loads `ulem`
   without `normalem`, `--marked` may alter emphasis rendering. That is a defect in a review artefact, not
   in a submission, and it is why bound 1 exists.

## What was rejected

**Kernel-only marking.** `\underline` exists; no kernel strikethrough does. Rendering deletions as
`[deleted: …]` was considered and rejected: it makes a long deleted paragraph unreadable, which defeats the
view's only purpose.

**Requiring the author to load the packages.** It moves the injection into the source, where it would then
appear in the submitted `.tex` — strictly worse. The build that must stay clean would become the one
carrying review machinery.
