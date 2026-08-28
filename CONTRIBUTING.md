# Contributing to NextTeX

Orientation, invariants and anti-patterns are in [`AGENTS.md`](AGENTS.md). Positioning and the language
allowed for claims are in [`PHILOSOPHY.md`](PHILOSOPHY.md). **This file covers setup, tests and the traps
specific to working here** — it does not restate the other two.

---

## Local stack

Rust via [`rustup`](https://rustup.rs). Nothing else is installed globally.

```bash
cargo test                              # the invariants
cargo clippy --all-targets -- -D warnings
cargo fmt --check
```

Run clippy over the **workspace**, not one crate: CI does.

A TeX distribution is needed for the tests that build documents. The engine is pinned — an invariant that
compares rendered pages is meaningless against a moving typesetter.

---

## Tests

Three suites, and they check different things.

**Invariant tests.** One per invariant in [`AGENTS.md`](AGENTS.md) §4, written so a failure names the claim
that stopped being true: *untouched LaTeX comes out byte-identical*, *opaque bytes are never normalised*, *a
hard error only comes from an explicit construct*. If one starts failing, fix the code, not the test.

**Corpus tests.** Real `.tex` files with declared provenance and licence, plus synthetic adversarial files
for every hazard in `PHILOSOPHY.md` — `\verb`, `verbatim`, `lstlisting`, `\catcode`, `\makeatletter`,
`\newenvironment`, unbalanced conditionals. Each entry compares three things: the emitted `.tex`, the
diagnostics, and the exit code.

**Property tests for the gradual guarantee.** Add valid annotations at random eligible positions, rebuild,
and require that the rendered pages are identical and the build status is unchanged. This is the suite that
catches an emitter which "improves" what it was supposed to transport.

**The rule that matters here more than coverage:** any number this repo publishes — a coverage figure, a
transport rate, a runtime — has a documented command that reproduces it. A number without that command does
not go in a README, a PR body, or a report.

---

## Before you open a PR

- [ ] `cargo test`, `cargo clippy`, `cargo fmt --check` are clean
- [ ] The corpus still transports byte for byte — no file needed a special case
- [ ] Every number you report has the command that reproduces it, in the test plan
- [ ] The invariants in [`AGENTS.md`](AGENTS.md) §4 still hold
- [ ] The diff went past a decorrelated model arm, and what it found is in the PR body or dismissed with a
      reason
- [ ] The documentation the issue named is updated **in this PR**

---

## Gotchas

- **The emitter must not be helpful.** Reindenting, collapsing whitespace or reordering arguments inside an
  opaque region is a silent corruption of somebody's accepted paper. If a change makes the output "nicer",
  it is a bug.
- **`?O` is not "invalid".** Ordinary LaTeX has an unknown *open* type and is consistent with everything.
  Making it fail is not stricter checking, it is a broken promise: a renamed `.tex` must check clean.
- **A structural check has to survive the constructs that legitimately break it.** Count columns without
  summing `\multicolumn` widths and the checker reports false positives on ordinary tables.
- **A hazard has to be reproduced before it is handled.** The parser's behaviour on `\catcode` is decided by
  what the corpus shows, not by what seems safe. If the measurement has not been made, the construct goes to
  `OpaqueToEof` and the issue says so.
- **Prior art is checked against live sources, not recalled.** The positioning in `PHILOSOPHY.md` §3 rests on
  a novelty check with a stated boundary. Any claim that goes beyond it needs its own check first.

---

## Pointers

- [`PHILOSOPHY.md`](PHILOSOPHY.md) — what NextTeX is, what it is not, and what may be claimed. Binding.
- [`AGENTS.md`](AGENTS.md) — orientation, invariants, anti-patterns, workflow. **Read before non-trivial
  changes.**
