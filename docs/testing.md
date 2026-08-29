# Test strategy

A compiler whose promise is "your bytes come back unchanged" cannot be tested by a list of examples. The
guarantees are universally quantified, so most of the suite generates its own cases.

Six layers. Each catches a class of failure the others structurally cannot.

---

## 1 · One test per invariant, named after the claim

Every invariant in [`AGENTS.md`](../AGENTS.md) §4 has exactly one test, and the test is named so that a
failure reports which claim stopped being true — not which function returned the wrong value.

```
untouched_latex_comes_out_byte_identical
annotating_never_changes_the_rendered_page
annotating_never_turns_a_passing_build_into_a_failing_one
nothing_is_injected_into_emitted_output
opaque_bytes_are_never_normalised
no_hard_error_originates_outside_an_explicit_construct
unknown_latex_is_never_fatal
every_diagnostic_names_its_blame_side
an_entry_token_cannot_appear_in_ordinary_prose
```

**Rule.** If one of these fails, fix the code. Never the test. A test in this layer that is edited to pass is
an invariant being quietly dropped, and the invariants are the product.

---

## 2 · Grammar conformance — the examples are executable

Every production in [`grammar.md`](grammar.md) carries a valid fixture and an invalid one, and both are run.
The §9 example blocks are not illustration; they are the fixture list.

Three assertions per fixture:

| Fixture kind | Asserted |
|---|---|
| Valid | parses to the specified tree, and to exactly one tree |
| Invalid | produces the specified diagnostic code, at the specified span |
| Not-a-construct | produces **no** diagnostic, and the bytes are unchanged |

The third kind is the one that is easy to omit and expensive to lose. `@` inside `\email{}`, inside a
comment, inside `\makeatletter`, inside a listing — each is a fixture asserting that ExactTeX left it alone.

A construct having two valid parses is a bug, and it stays invisible until every production has a fixture
pinning the byte at which it ends.

---

## 3 · Property tests — the core, and they start in Phase 2

The two correctness properties are quantified over all inputs, so they are tested by generation.

### Transport

```
for any byte sequence u containing no ExactTeX construct:
    emit(parse(u)).tex == u
    check(parse(u)) has no hard errors
```

Generators: arbitrary bytes; valid LaTeX fragments; entry tokens placed in every non-prose position; every
hazard construct; and **truncation at every byte boundary** of each of the above. Truncation is where
unterminated `\verb`, unmatched `\makeatletter` and half-written constructs come from, and it is the cheapest
generator that finds real bugs.

### Typesetting equivalence

```
for any document d that passes checking:
    render(tex(emit(d))) == render(tex(emit(erase(d))))
    tex(emit(d)).status  == tex(emit(erase(d))).status
```

Generator: take a corpus document, insert valid annotations at randomly chosen eligible positions, build both
variants, compare.

**Before the comparison tolerance can be set, the renderer's own noise must be measured**: build the same
unchanged document repeatedly and record how much its rasters differ. A tolerance chosen before that
measurement is a number invented to make the suite pass.

### Sequencing — a correction to the phase plan

`ROADMAP.md` currently places property testing in Phase 5. **The transport property test must exist as soon
as the emitter exists, in Phase 2.** A property that is only checked at the end turns qualification into
archaeology: every regression between Phase 2 and Phase 5 has to be located after the fact, in code nobody is
holding in their head any more.

Phase 5 keeps the expensive half — full fuzzing campaigns, rendered comparison at scale, native/WASM
conformance. The cheap half runs from the first commit that emits a byte.

---

## 4 · Corpus and golden files

Real `.tex` files with declared provenance, licence and fingerprint, plus synthetic adversarial files. Each
corpus entry pins three things, and a change to any of them is a reviewable diff:

- the emitted `.tex`, byte for byte;
- the diagnostics, with codes and spans;
- the process exit code.

**The negative corpus matters as much as the positive one.** Files that must produce *zero* ExactTeX
diagnostics — ordinary papers with unresolved `\ref`, undefined citations, exotic packages — are what keep
the "renamed `.tex` checks clean" promise honest. A checker that gets stricter over time fails here first.

Its first form is `ordinary_latex_yields_no_constructs` in `crates/xtex-core/tests/fixtures.rs`. It found
[#39](https://github.com/camilochs/exacttex/issues/39) on the day it was written: `\lstinline|@id(x)|` was
being read as syntax, in a sentence whose whole point was to write the token without using it.

**Every fixture's `scanner:` line is checked.** Each `expect.txt` carries a strict line — `none`, or a list
where `ref` is a construct and `!ref` a malformed one — and `every_fixture_produces_the_pieces_it_declares`
compares it against what the scanner produced. Before that test existed, all 42 fixtures declared their
constructs in prose and nothing read it: the grammar was documented and unfalsified at the same time.

The `scanner:` lines were transcribed from each fixture's own prose, never from what the scanner did. Where
the two disagreed, the grammar decided: it found one real scanner defect (a signature trusted through a
mismatch) and one wrong fixture input (`latex{}`, which §7 does make a raw escape). A `scanner:` line
written to match the code would have documented both as correct.

---

## 5 · Hazard fixtures — the falsifying observation is the test

The hazard table in [`ROADMAP.md`](../ROADMAP.md) already carries, for each construct, the observation that
would show its handling is wrong. Each of those becomes a test directly:

| Hazard | The test asserts |
|---|---|
| `\verb` with an unusual delimiter | bytes inside are not parsed as ExactTeX |
| `\verb` unterminated | the file goes to `OpaqueToEof`; no delimiter is invented |
| `verbatim`, `lstlisting` | a marker inside is not a construct; no contained byte changes |
| `\catcode` at top level | parsing does not resume past the boundary |
| unmatched `\makeatletter` | no native syntax is recognized after it |
| `\newcommand` body containing `@ref(` | it stays opaque and produces no hard error |
| `\if` branches | both are preserved; neither is evaluated |
| `\input` | normal output contains no inlined bytes |

**Monotonicity of quarantine is its own test:** once a file has entered `OpaqueToEof`, no later construct is
recognized. It is a one-line property and it protects a boundary that is easy to erode by accident.

---

## 6 · Differential testing

Two implementations of the same thing, compared:

- **native against WASM** — same fixtures, same emitted bytes, same JSON diagnostics;
- **annotated against erased** — the typesetting-equivalence property, above;
- **CLI against LSP** — identical input must yield identical codes, spans, severities, entities and blame.
  Two renderers over one diagnostic model; if they diverge, the model is not actually shared.

---

## What this strategy does not cover

- **Whether the syntax is the right syntax.** That is a design decision, not a measurement. What Phase 0
  counts is different: how many of a real paper's figures, tables, sections and citations the author ends up
  declaring, since a diagnostic can only name what was declared.
- **Whether a translated diagnostic is understandable.** "Your table runs past the margin" being clearer than
  `Overfull \hbox` is a judgement about people. It can be reviewed; it cannot be asserted.
- **TeX's own correctness.** The engine is pinned and treated as given. If it changes, the rendered
  comparisons are rebaselined deliberately, with the engine fingerprint recorded.
