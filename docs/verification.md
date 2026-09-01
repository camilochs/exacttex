# External verification

What the compiler can and cannot know about the world outside the document, and how the two are kept
apart.

A document makes claims the compiler cannot decide by reading the document: that a bibliography entry
names a real publication with those authors and that year, that a URL answers, that a DOI resolves, that
a repository exists. Deciding them takes the network, and the network breaks the two promises the rest of
this compiler is built on — determinism (the same input giving the same answer) and speed (a check that
never waits on anything). So verification lives **behind its own door**: a separate step, a separate
binary, and a dated record as the only thing that crosses back.

The design in one line: **the compiler lists the claims and replays the verdicts; only `xtex-verify`
touches the network, and never during a compile.**

---

## 1 · The claims inventory

```sh
xtex claims paper.xtex
```

Deterministic and offline. The compiler walks the project and lists every external claim with the exact
span it was written at:

| Kind | What counts |
|---|---|
| `bib-entry` | every declared bibliography entry, with its fields (title, author, year, doi…) |
| `url` | every readable `url`/`href` value in typed blocks |
| `doi` | a `doi.org` address, classified by what it claims rather than where it appears |
| `repository` | an address on a software forge (GitHub, GitLab, …) |

The output is JSON: `{"claims":[{"kind":…,"target":…,"file":…,"offset":…,"length":…,"fields":{…}}]}`.
This is the whole interface between the compiler and any verifier — the terminal one in this repository,
the browser one in an editor, or one that does not exist yet.

## 2 · The verifier

```sh
xtex-verify paper.xtex [--max-age=30d] [--mailto=you@example.org]
```

`xtex-verify` is a separate crate, deliberately **excluded from the workspace**, so the compiler's
zero-dependency invariant stays byte-true — its lock file never learns that an HTTP client exists. It
asks the open sources about each claim and writes the record beside the root document:

- **Bibliographic entries with a DOI** go to OpenAlex, batched up to fifty DOIs per request.
- **Entries without a DOI** go to Crossref's bibliographic search, matched by normalized title.
- **URLs, DOIs and repositories** are probed directly; a redirect is an answer, not an error.
- A **fixed token bucket per host** keeps every request inside the sources' polite-pool limits, and a
  **retry budget** (three attempts per request, globally at most one retry per ten requests) means a dead
  network costs bounded time instead of a storm.
- The run is **incremental**: an entry that is unchanged, answered and inside the freshness window is
  carried over from the previous record with zero network. Edit one entry among two hundred and the next
  run pays one request.
- Every run **measures itself** — requests per source, retries, bytes, fetched / carried over /
  unanswered — because a measurement that cannot say what it did is a measurement nobody can compare.
- The record is **persisted after every claim settles**, so an interrupted run keeps everything it
  obtained and the next run completes the holes.

A timeout is never conflated with nonexistence: a claim the network failed to answer is recorded as
unanswered **with the failure note**, and is always retried on the next run.

## 3 · The record

The verifier's output is `.xtexverified`, a JSON file beside the root document. Every verdict in it
carries three things that make it evidence rather than opinion:

- **A date** (`fetched_at`). "The repository existed" is not a fact; "the repository answered on
  2026-09-01" is. Verdicts expire: past the freshness window (default 30 days) they retire to advisories.
- **A fingerprint of the document's own fields** at the time of the check. Edit the entry and its verdict
  retires automatically — it was evidence about a text that no longer exists.
- **A source and a response fingerprint** — which source answered and a hash of what it said, so a later
  run can ask "did the answer change" without trusting anyone's memory, including its own.

Verdicts come in two vocabularies, never mixed:

| Claim | Verdicts |
|---|---|
| `bib-entry` | `verified` · `partial` (fields differ, title holds) · `mismatch` (title differs) · `unverified` (no source answered) |
| `url` / `doi` / `repository` | `reachable` · `redirected` · `unreachable` |

A `partial` or `mismatch` carries per-field diffs with both sides quoted. **Author diffs are always high
severity**: the classic fabricated reference has a correct title and a working DOI with an invented
author list, and the working DOI masks the error.

Parsing is whole-or-nothing: a record with an unknown verdict, a missing date, or a failure without its
note is refused entirely rather than half-read. A refused record never breaks a build — the check reports
it could not read the record and carries on.

## 4 · Replaying the record

```sh
xtex check --verified paper.xtex          # reads .xtexverified beside the root
xtex check --verified=path/to/record …    # or a named record
```

Offline, deterministic, and exactly as fast as a plain check. The compiler reads the record and restates
its findings as ordinary diagnostics at the claim's own span — a mismatch on the entry's line, a dead URL
on the URL's line, each wording dated. Without `--verified`, the check is byte-for-byte what it always
was.

Severity follows the gradual policy of the rest of the language: a finding is a hard error only where the
author opted in (an `@cite` demanding the entry, met by a `mismatch` or a high-severity diff); everything
else — reachability, expiry, drift — is advisory. The WebAssembly surface exposes the same two calls
(`xtex_claims`, `xtex_check_with_record`), so an editor in a browser replays the same record with the
same answers; the parity suite holds it to that.

## 5 · The provider: offering instead of checking

```sh
xtex-verify materialize 10.1000/xyz [--key=name]
```

The constructive half. Instead of checking an entry someone typed, the provider **transcribes** the
entry from the DOI's live record and prints it, dated:

```bibtex
% transcribed from crossref on 2026-09-01 — doi:10.1000/xyz
@article{lovelace2021,
  title = {A Very Exact Result},
  author = {Ada Lovelace and Alan Turing},
  ...
```

An entry nobody typed cannot carry an invented field — the classic fabricated reference is composed
from memory, and this path has no memory to compose from. A DOI the source does not know is a refusal,
never a guess; a record without a title is refused, never completed. Editors expose the same operation
(in Vitela: *Add entry from DOI* in the submission report), and once the entry is in the project the
deterministic check owns it like any other.

## 6 · What this refuses to be

- **Not a compile-time network.** `--verify-external` inside the pipeline was considered and rejected:
  hundreds of citations would make compilation minutes long, and a timeout would turn weather into build
  breakage.
- **Not part of the PDF.** The record is a project sidecar; nothing about verification is written into
  emitted LaTeX.
- **Not a truth oracle.** A verdict is what a named source said on a dated day, fingerprinted. The record
  never claims more, and the expiry makes sure it stops claiming it on time.
