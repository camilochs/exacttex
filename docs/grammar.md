# NextTeX grammar

**Status: draft.** This is the specification the hand-written parser is tested against. It is not generated
from, and no parser generator produces it — see [Why no parser generator](#why-no-parser-generator).

Binding context: [`PHILOSOPHY.md`](../PHILOSOPHY.md) §4 (design rules) and §5 (correctness properties);
[`AGENTS.md`](../AGENTS.md) §4 (invariants).

---

## 1 · Notation

```
name        a production
"literal"   an exact byte sequence
[a-z]       a byte class
x?          optional
x*          zero or more
x+          one or more
x | y       alternatives
(x y)       grouping
⟨…⟩         prose description of a byte set
```

Byte-oriented, not character-oriented. Input is a byte sequence and is never assumed to be valid UTF-8;
identifiers and keywords are ASCII, everything else passes through untouched.

---

## 2 · The document is LaTeX

There is no top-level NextTeX grammar. A `.ntex` file **is** a LaTeX byte stream, and NextTeX constructs are
recognized inside it at specific positions.

```
document    = ( ntex-construct | latex-bytes )*
latex-bytes = ⟨any bytes not beginning a recognized ntex-construct in prose position⟩
```

`latex-bytes` are never interpreted, normalized or validated. They are transported. This is the transport
property in `PHILOSOPHY.md` §5.

---

## 3 · Entry tokens

Only two byte sequences can begin a NextTeX construct:

```
ntex-construct = at-construct | block-construct

at-construct    = "@" keyword "(" …
block-construct = "\" block-keyword "(" …
```

Both are chosen so that no valid existing `.tex` can contain them by accident:

- `@` followed by a NextTeX keyword and `(`. Measured against a corpus of six published papers (8 029 lines):
  23 occurrences of `@` followed by a letter, **none in prose position** — all inside `\email{}` / `\ead{}` /
  `\texttt{}` arguments, between `\makeatletter` and `\makeatother`, or inside a code listing.
- `\figure(` and `\table(`. In plain LaTeX these are undefined control sequences followed by `(`, so no
  valid document contains them. If the document itself defines `\figure` or `\table`, the checker reports a
  conflict — the collision is detected, never silent.

**Rule (binding).** A construct is recognized only in **prose position**: outside math, outside verbatim-like
environments, outside command arguments, outside `\makeatletter … \makeatother`, and outside any region the
parser has marked opaque. See §8.

---

## 4 · Inline constructs

```
at-construct = id-decl | reference | citation | import | revision

id-decl   = "@id"    "(" ident ")"
reference = "@ref"   "(" ident ")"
citation  = "@cite"  "(" bibkey ( "," ws* field )* ")"
import    = "@import" "(" string ")"

ident   = [A-Za-z] [A-Za-z0-9_:.-]*
bibkey  = [A-Za-z0-9] [A-Za-z0-9_:.+/-]*
```

`@id` attaches to the LaTeX construct that **precedes** it on the same line, or to the environment whose
`\begin` precedes it. It declares an entity without NextTeX knowing what kind of construct it is — this is
what makes theorems, algorithms and anything else referenceable with no new syntax.

```latex
\section{Introduction} @id(sec:intro)

\begin{theorem} @id(thm:bound)
  ...
\end{theorem}
```

`@cite` accepts fields, so that a rendering distinction is never carried by a single character:

```latex
@cite(knuth1984)                      parenthetical
@cite(knuth1984, style=textual)       the citation is the sentence subject
```

**Boundary.** Every inline construct ends at its matching `)`. Nesting of parentheses inside `ident`,
`bibkey` or a field value is not permitted; a `)` always closes. An unterminated construct is a hard error
reported at the opening token, and no bytes are consumed past end of line.

---

## 5 · Block constructs

```
block-construct = block-keyword "(" ident ")" ws* "{" field-list "}"
block-keyword   = "\figure" | "\table"

field-list = ( ws* field ws* )*
field      = key ws* "=" ws* value
key        = [a-z] [a-z0-9_]*

value = string | length | percentage | integer | braced | bare
```

```latex
\figure(fig:runtime) {
  src     = "figures/runtime.pdf"
  width   = 80%
  caption = Runtime architecture for \emph{multi-agent} systems
}
```

### Value kinds

| Kind | Form | Ends at | Used for |
|---|---|---|---|
| `string` | `"…"` with `\"` escape | closing quote | paths — a path may contain spaces |
| `length` | number + unit (`pt`, `mm`, `cm`, `in`, `em`, `ex`) | end of token | explicit dimensions |
| `percentage` | number + `%` | end of token | relative widths |
| `integer` | digits | end of token | counts, e.g. `columns` |
| `bare` | any bytes | end of line | prose that contains LaTeX — captions |
| `braced` | `{ … }` balanced | matching `}` | multi-line prose or table bodies |

**Why `bare` runs to end of line and is not a quoted string.** A real caption contains LaTeX: `$\alpha$`,
`\emph{}`, sometimes a citation. A quoted string cannot hold it without an escaping scheme nobody wants to
write. A caption longer than one line uses the `braced` form.

**Boundary.** A block ends at the `}` matching its opening `{`, counted with LaTeX's own escaping rules:
`\{` and `\}` do not count, and `%` starts a comment to end of line. A block whose braces do not balance
before end of file is a hard error.

---

## 6 · Revision constructs

```
revision = add | del | sub | note

add  = "@add" "(" ident ")" ws* braced
del  = "@del" "(" ident ")" ws* braced
sub  = "@sub" "(" ident ")" ws* "{" content "->" content "}"
note = "@note" "(" ident "," ws* "on" ws* "=" ws* ident ")" ws* braced
```

Metadata — author, timestamp, status, thread — lives outside the document, in a sidecar keyed by the
identifier. See [`revisions.md`](revisions.md).

**Constraints (v0.1).** Content must be brace-balanced. Revisions must not overlap; an overlap is a hard
error, not a merge. A `@sub` is one change, not a delete plus an insert, so accepting it is atomic.

---

## 7 · The raw escape

```
raw = "latex" ws* "{" ⟨balanced bytes⟩ "}"
```

```latex
latex {
  \begin{tikzpicture} \draw (0,0) -- (2,1); \end{tikzpicture}
}
```

Contents are transported. No NextTeX construct is recognized inside. Everything inside counts as unchecked
against the coverage figure reported by `nextex check`.

---

## 8 · Where constructs are not recognized

This section is the difference between a language and a text substitution. In each region below, the byte
sequence `@ref(` is ordinary text.

| Region | Begins at | Ends at |
|---|---|---|
| Comment | unescaped `%` | end of line |
| Inline math | `$` | matching `$` |
| Display math | `$$` or `\[` | `$$` or `\]` |
| Verbatim command | `\verb` + delimiter byte | next occurrence of that byte |
| Verbatim environment | `\begin{verbatim}` | line-exact `\end{verbatim}` |
| Listing | `\begin{lstlisting}` and names from `nextex.toml` | its line-exact terminator |
| Internal-macro region | `\makeatletter` | `\makeatother` |
| Raw escape | `latex {` | matching `}` |
| Quarantine | see below | end of file |
| Command argument | `{` of a known argument-taking command | matching `}` |

### Quarantine

When the parser cannot bound a region safely it stops recognizing NextTeX constructs rather than guessing.

```
ParseConfidence = Structured | OpaqueBalanced | OpaqueToEof
```

`OpaqueToEof` is entered by an unterminated `\verb`, an unmatched `\makeatletter`, a `\catcode` assignment at
top level, and any construct whose end cannot be located. **It is monotonic**: once entered, no later
construct in that file is recognized. The full hazard table is in [`../ROADMAP.md`](../ROADMAP.md).

---

## 9 · Examples

### Valid

```latex
\section{Model} @id(sec:model)                          declaration on a section
\begin{theorem} @id(thm:x) ... \end{theorem}            declaration on any environment
As shown in @ref(fig:runtime), ...                      reference in prose
@cite(knuth1984, style=textual) proved that ...         citation with a rendering field
@import("sections/model.ntex")                          path relative to THIS file
\figure(fig:r) { src = "r.pdf"  width = 80% }           block on one line
```

### Invalid, and what is reported

```latex
@ref(fig:runtime                    unterminated construct — hard error at "@ref"
@ref()                              empty identifier — hard error
\figure(fig:r) { src = r.pdf }      unquoted path — hard error, paths are strings
\figure(fig:r) { width = 80 }       bare number where a length or percentage is required
@id(sec:x) ... @id(sec:x)           duplicate identifier — hard error, both spans reported
@sub(c1) { a -> b -> c }            more than one arrow — hard error
```

### Not a construct — ordinary text, no diagnostic

```latex
\email{camilo.chacon@icn2.cat}      @ inside a command argument
\makeatletter \let\@oddhead\@empty  @ inside the internal-macro region
% pending: @ref(fig:x) needs a name  @ inside a comment
\begin{lstlisting}
  engine: "llama.cpp@a1b2c3"        @ inside a listing
\end{lstlisting}
```

---

## 10 · Why no parser generator

The parser is hand-written recursive descent, with a hand-written lexer. Three reasons, in order of weight:

**The hard part is not context-free.** `\verb`'s delimiter is whatever byte follows it. Verbatim
environments consume raw lines. `\catcode` changes what a byte means. `\makeatletter` changes what counts as
a letter inside a control-sequence name. None of that is expressible as a grammar; in a generator it becomes
semantic predicates and lexer modes — the same logic written by hand, inside someone else's framework.

**Generators discard what must be preserved.** They build their own tree over decoded strings and normally
drop whitespace and comments. The transport invariant requires every byte and a span indexing an immutable
buffer.

**Error recovery is a designed behaviour here, not a fallback.** Quarantine — degrade to opaque and keep
going, never reject — is specific to this compiler. A generator supplies its own recovery.

ANTLR specifically has no official Rust target; the community effort is not a dependency this project takes
on. For the native syntax alone a PEG generator would work, but that yields two parsers in two styles while
the difficult one stays hand-written.

This is also what production compilers do: rust-analyzer, Clang and TypeScript all use hand-written
recursive descent, and GCC's parser performance improved when it moved off a generated parser.

---

## 11 · Open questions

- **`needs` and the no-injection rule.** The block form accepts `needs = <package>`, and package synthesis
  writes `\usepackage` into the preamble. That is generated output, which `PHILOSOPHY.md` §5 forbids.
  Unresolved; tracked as repository issue #5. The grammar above admits the field without settling what it
  emits.
- **Entity kinds beyond figure and table.** `@id` covers everything at the light level. Whether equations,
  algorithms or theorems get typed blocks is deferred until a checked field is demanded for them.
- **Argument-position detection.** §8 excludes command arguments, which requires knowing which commands take
  arguments. The workable rule — treat a `{ … }` group following any control sequence as argument position —
  needs validation against the corpus before it is frozen.
