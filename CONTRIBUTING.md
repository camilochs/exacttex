# Contributing to ExactTeX

Orientation, invariants and anti-patterns are in [`AGENTS.md`](AGENTS.md). Positioning and the language
allowed for claims are in [`PHILOSOPHY.md`](PHILOSOPHY.md). **This file covers setup, tests and the traps
specific to working here** — it does not restate the other two.

---

## What is open, and what is not

The project is open by zone. The reason is not ceremony: the compiler holds a
few invariants that hold the whole design up — untouched LaTeX comes out
byte-identical, opaque bytes are never rewritten, one table decides where the
document carries text — and those are easy to break by accident and expensive
to discover later. So the closer a change sits to them, the more this project
asks of it.

**Open, and the most useful thing you can send.**

- **Documents that break it.** A `.tex` or `.xtex` file that the compiler
  mishandles, with what you expected and what you got. Licence and provenance
  stated, so it can live in the corpus. This is worth more to the project than
  any patch: every real document so far has found something.
- **Bug reports** of any kind, especially with a minimal reproduction.
- **Documentation, examples, the site, editor integrations, tooling around the
  compiler.** Pull requests welcome, reviewed on their own merits.

**Open with a contract: `xtex-cli`, `xtex-lsp`, `xtex-wasm`, diagnostics.**
Pull requests welcome, and each one carries a test that would fail without it.
A change that alters what the compiler *says* — a diagnostic's wording, a code,
an exit status — is a change to an interface other tools read, so say so in the
description.

**Closed for now: `xtex-core`'s scanner, document model and emitter.**
These are where the invariants live. Issues are read and answered; pull requests
against them are likely to be declined, not because the idea is unwelcome but
because the design is still moving under them and a merged change would freeze
a decision that is not made yet. If you have one, open an issue first and it
will get a real answer.

This boundary is meant to move. When the core stops changing weekly, it opens.

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
  Making it fail would break a promise rather than tighten the checking: a renamed `.tex` must check clean.
- **A structural check has to survive the constructs that legitimately break it.** Count columns without
  summing `\multicolumn` widths and the checker reports false positives on ordinary tables.
- **A hazard has to be reproduced before it is handled.** The parser's behaviour on `\catcode` is decided by
  what the corpus shows, not by what seems safe. If the measurement has not been made, the construct goes to
  `OpaqueToEof` and the issue says so.
- **Prior art is checked against live sources, not recalled.** The positioning in `PHILOSOPHY.md` §3 rests on
  a novelty check with a stated boundary. Any claim that goes beyond it needs its own check first.

---

## Pointers

- [`PHILOSOPHY.md`](PHILOSOPHY.md) — what ExactTeX is, what it is not, and what may be claimed. Binding.
- [`AGENTS.md`](AGENTS.md) — orientation, invariants, anti-patterns, workflow. **Read before non-trivial
  changes.**
