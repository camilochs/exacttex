# What the corpus says about the grammar

Measured across 111 `.tex` files from the author's published papers. Every number below is reproducible with
the scripts noted; each one contradicts or confirms something the draft grammar assumes.

---

## 1 · `@id` must attach across a newline

`docs/grammar.md` §4 says `@id` "attaches to the LaTeX construct that **precedes it on the same line**".

That is wrong for the dominant case.

| Where `\label` sits relative to `\caption` | Count |
|---|---|
| Same line | 9 |
| **Next line** | **442** |

Of 479 captions, 442 are followed by their label on the following line. Any rule requiring same-line
adjacency fails on 92% of the figures and tables in this corpus.

The attachment rule has to be "the preceding construct", with whitespace and newlines skipped — and the
grammar has to say how far it will skip before giving up.

## 2 · A caption is not a to-end-of-line value

The draft admits a `bare` value kind that "runs to end of line", and names `caption` as its use.

| Property of real captions (n = 479) | Value |
|---|---|
| Median length | 230 characters |
| Longest | 1 160 characters |
| **Containing a newline** | **111 (23%)** |
| Containing nested braces | 207 |

Nearly a quarter of captions span more than one line. `bare` cannot be the default for `caption`; the braced
form has to be, or the rule has to become "to the end of the balanced group".

## 3 · Brace counting must not treat `\%` as a comment

Inside caption arguments: **120 escaped `\%`, and zero real comments.** The percent sign appears in captions
as text — percentages in results — and never as a comment.

The grammar says `%` starts a comment when counting braces. It must also say `\%` does not. Getting this
wrong truncates a caption at the first percentage figure.

## 4 · Revision markup is mostly inline, and it nests

`\textcolor{blue}{...}`, the author's revision convention, appears 179 times:

| Shape | Count |
|---|---|
| **Inline, within a sentence** | **175** |
| Multi-line | 4 |
| Longest block | 2 040 characters |
| **Containing `\ref`, `\cite` or `\label`** | **33** |

Two consequences for `@add` / `@del` / `@sub`:

- The dominant use is a phrase inside a sentence, not a block between paragraphs. The constructs must work
  mid-sentence.
- 33 of them contain references or citations, so a revision construct must be able to **hold other ExactTeX
  constructs inside it**. The draft grammar's `braced` content does not say whether nested constructs are
  recognized there.

## 5 · Entry-token safety, confirmed and corrected

Every occurrence of `@` in the corpus, by the character that follows it:

| Shape | Count | Where |
|---|---|---|
| `@{` | **322** | tabular column specifications |
| `@` + letter | 143 | email addresses, `\makeatletter` internals, one string in a listing |
| `@` + space | 1 | |

`@{` is the **most common shape by more than two to one**, and the earlier count that justified the `@` entry
token searched only for `@` + letter and did not see it. It does not collide — the entry token requires
`@` + keyword + `(` — but the grammar should state this rather than leave a reader to derive it.

**No file defines `\figure` or `\table`.** Checked across all 111. The block entry token is free.

## 6 · Volume of what still has no way to be named

| Construct | Occurrences |
|---|---|
| `algorithm` environments | 49 |
| Numbered lines inside them (`\STATE`) | 237 |
| `subfigure` environments | 34 |

Algorithms are not an edge case in this corpus; they are more common than the theorem environments the draft
already accommodates. Subfigures are frequent enough that "a panel inside a subfigure" — listed as a gap in
`GAPS.md` — will be hit early.

---

## Reproduce

The measurements are single-pass regex sweeps over the corpus, run from the research directory. The caption
measurement extracts balanced arguments rather than matching with a regex, because 207 captions contain
nested braces and a naive `\caption\{([^}]*)\}` truncates them.

## Boundary

One author, 111 files, recent work. A corpus from other authors would shift the numbers and may add shapes —
in particular the `@` survey and the `\figure` collision check are only as good as this sample. Neither is a
claim about LaTeX in general.
