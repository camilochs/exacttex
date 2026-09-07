# 0021 · A row found by search cannot stop the build

**Status:** accepted, 2026-09-07. Decided by the maintainer after a correct entry refused a build
(issue #183).

---

## The gap this closes

`lamport1994latex` — Lamport's book, second edition, 1994, with its ISBN — has no DOI, so the verifier
asked Crossref's bibliographic search and took the top row with the same title. That row is a review
of the book in *Biometrics* (1996, seven authors). The record said `partial`, authors differing at high
severity; the key is demanded by an `@cite`; `xtex check --verified` refused the build. The entry was
right, and Open Library confirms it as written.

## The rule

`XT1018` is a hard error only when the record was reached through an identifier the entry itself
declares — today a DOI, answered by OpenAlex. A record reached by text search (`crossref-query`) keeps
its verdict and its diffs, but the finding is an advisory, and its message says the row was found by
search, so the reader knows the difference may be between two works.

The hard error exists for one shape: a fabricated author list behind a correct DOI. That inference
needs the row to be the entry's own. A row found by searching is the verifier's guess about which work
the entry means, and a difference against a guess may be a difference between two works.

## What this costs

A wrong author list on a DOI-less entry no longer stops the build; it is still reported, with both
sides, on every check. The maintainer accepted this: the verifier must not be more confident than its
lookup.

## What was considered and not taken

Open Library by ISBN as a source for books. Checked live on the same day: its record for
ISBN 0-201-52983-1 carries the title `LATEX`, which would turn the correct entry into a title mismatch.
A source that can stop a build has to be at least as careful as the check.
