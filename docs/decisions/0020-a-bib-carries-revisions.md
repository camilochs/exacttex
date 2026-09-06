# 0020 · A `.bib` carries revisions, and its readers see the final view

**Status:** accepted, 2026-09-06. Decided by the maintainer after a proposal written into a `.bib` by the
agent bridge.

---

## The gap this closes

The bridge proposed a bibliography entry as an `@add` in `refs.bib`. The host's margin shows no card on a
`.bib`, so nobody could accept, reject or withdraw it; the sidecar, named by replacing `.xtex` only, was
written into the `.bib` itself; and the check, meeting `@sub(change:x) {…}` where an entry should begin,
reported the bibliography unparsable — which, by `docs/checking.md`, silences every `XT1005`. A wrong
citation key went unreported while the file looked complete.

## The decision

**A revision construct is legal in a `.bib`, and every reader of a `.bib` reads the final view of its
bytes.** `review::view_bytes(bytes, view)` resolves every construct the scanner recognises — reject all
for the original, accept all for the final — by the same `resolve` the margin uses, so nesting and
replies behave as a decision would. The check's key collection, the claims inventory and the wasm text
view (`xtex_text_view`) go through it.

Final, not original, because the sources are checked that way: a proposed citation is checked against
the text it would produce, so the entry it cites must count once proposed.

## What this does not decide

The CLI does not rewrite a `.bib`: an author running BibTeX by hand resolves pending revisions first
(`xtex revise`). Whether `xtex build` should emit a `.bib` view beside the `.tex` is open, and it is a
host-side question until a CLI workflow needs it.
