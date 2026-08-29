# What a broken `.bib` looks like, and who notices

`#64` made the compiler say when it could not *read* a bibliography. It still says nothing when it reads one
that is *broken*. This measures what it would take to notice, and what it would cost.

## Ground truth is BibTeX, not judgement

Whether a `.bib` file is "valid" is not a question about the BibTeX grammar in the abstract. It is a question
about the program that consumes it. `groundtruth.py` runs each file through **BibTeX 0.99e** over a minimal
`.aux` that cites everything, and records whether it reported an error.

```sh
python3 groundtruth.py corpus/*.bib
```

Result, BibTeX 0.99e (TeX Live 2026):

| Files | Verdict |
|---|---|
| `bad-01` … `bad-04` | rejected — `Illegal end of database file`, `I was expecting a ',' or a '}'`, `Unbalanced braces` |
| `bad-05` … `bad-07` | **accepted**, with warnings — an empty key, a stray brace, an undefined `@string` |
| `ok-01` … `ok-07` | accepted |

The three `bad-` files BibTeX accepts are in the corpus deliberately. A validator that rejects them is wrong,
however reasonable the rejection looks.

## What the candidates do

The same 14 files, plus 37 real `.bib` files from the author's projects, through both crates and our own key
reader. Measured 2026-08-29.

| | Detects the 4 real errors | Falsely rejects a valid file | Message | New dependencies |
|---|---|---|---|---|
| our key reader | **0 of 4** | 0 | — | 0 |
| `nom-bibtex` 0.6, MIT | 4 of 4 | 0 of 44 | unusable | +12 crates, two versions of `nom` |
| `biblatex` 0.12, MIT OR Apache-2.0 | 4 of 4 | **3 of 44** | clear | +8 crates |

Licence was the first thing checked and it turned out not to be the constraint: every serious candidate on
crates.io is MIT or MIT/Apache-2.0. Three other things are.

1. **`nom-bibtex`'s message names its own combinators.** For an unclosed brace on line 3 of a 5-line file:
   `Parsing error. Reason: 0: at line 1, in IsNot: @book{knuth1984, ^`.
2. **No parser locates the author's mistake for an unclosed delimiter.** `biblatex` reports byte 96 of a
   96-byte file; BibTeX itself says `Illegal end of database file---line 5`. A delimiter that never closes is
   only detectable at end of input, so "parsing stopped here" is the strongest honest claim available.
3. **`biblatex`'s three false rejections include a real file.** `irace-package.bib`, shipped in the `irace` R
   package, opens with an `@preamble` concatenating brace groups with `#`; the crate expects a quotation mark
   and stops at byte 12. It also rejects an undefined `@string`, which BibTeX only warns about. Under
   [`checking.md`](../../../docs/checking.md) §7 each of those silences citation checking for a whole
   document.

`tectonic_engine_bibtex` (MIT) is BibTeX itself as a crate, and is the reason ground truth was available at
all. It is not a candidate: it needs an `.aux` and a `.bst`, and using it would mean running the program,
which the compiler does not do.

## What the corpus decided

The validator that came out of this is a detector, not a parser: it locates the four shapes BibTeX rejects
and nothing else. Finding keys and validating syntax stay separate jobs, because the key reader must never
fail — a failure silences citation checking for a whole document — while a validator only produces an
advisory and so can afford to be strict.

Two files in the corpus were added after the first attempt, because they refuted it.

**`@comment` is the one entry type BibTeX does not read.** It skips to the next `@` and resumes there. A
validator that checks the body instead rejects `ok-08`, which BibTeX accepts, and that is the expensive
direction of error: a false rejection costs a document its citation checking.

**A commented-out entry is still a database entry.** In `ok-09` a `@book` sits inside a `@comment{...}`
wrapper. Because BibTeX's skip stops at the next `@`, it reads that entry and resolves it when cited —
verified by citing `commented_out` and watching BibTeX not complain. A validator that skipped to the
comment's closing brace would lose the key, and a lost key turns a correct `@cite` into `XT1005` against an
author who did nothing wrong.

`bad-08` and `bad-09` are the other side of the same question: BibTeX *does* check brace balance inside
`@preamble` and `@string`, and rejects both. The three types are not interchangeable.

## Boundary

One run. The malformed shapes were chosen by hand, and a different set could reorder the table. The 37 real
files are one author's corpus. Eighteen further probes — CRLF endings, parenthesis-delimited entries, an `@`
inside a value, a URL with `%` and `=`, non-ASCII names, `#` concatenation — agree with BibTeX, and one of
them does not: BibTeX counts a backslash-escaped `\{` as an opening brace and rejects the file, where the
validator accepts it. That is a shape it does not detect, not a file it wrongly rejects. What the numbers support is a decision about this codebase, not a claim
about BibTeX parsers in general.
