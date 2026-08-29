# NextTeX grammar

**Status: corrected v0.1 specification.** This is the specification against which the hand-written parser is
tested. It is not generated from, and no parser generator produces it; see
[Why no parser generator](#10--why-no-parser-generator).

Binding context: [`PHILOSOPHY.md`](../PHILOSOPHY.md) §4 (design rules) and §5 (correctness properties);
[`AGENTS.md`](../AGENTS.md) §4 (invariants).

---

## 1 · Notation

```text
name        a production
"literal"   an exact byte sequence
[a-z]       a byte class
x?          optional
x*          zero or more
x+          one or more
(x y)       grouping
x{0,n}      between zero and n repetitions
x | y       alternatives
⟨…⟩         prose description of a byte set
```

The grammar is byte-oriented, not character-oriented. Input is a byte sequence and is never assumed to be
valid UTF-8. Identifiers and keywords are ASCII. Other bytes pass through unchanged.

Unless a production states otherwise:

```text
hws         = (" " | "\t")*
line-end    = "\n" | "\r\n" | end-of-file
ws          = " " | "\t" | "\n" | "\r"
```

A parser reads bytes from left to right. Balanced regions require a nesting counter but no lookahead beyond
the current byte. The resource limit for a nesting counter is implementation-configured; exceeding it is a
resource-limit error, not an error in ordinary LaTeX.

---

## 2 · The document is LaTeX

There is no top-level NextTeX replacement grammar. A `.ntex` file is a LaTeX byte stream in which NextTeX
constructs are recognized at the positions defined by this specification.

```text
document    = ( ntex-construct | latex-bytes )*
latex-bytes = ⟨bytes not beginning a recognized NextTeX construct at the current position⟩
```

`latex-bytes` are not normalized or validated. Their spans index the immutable source buffer, and emission
copies those slices byte for byte.

For input containing no NextTeX constructs:

```text
emit(parse(input)) == input
```

Unknown LaTeX is transported rather than rejected. A hard error can originate only in an explicit NextTeX
construct, an invalid annotation encoding, an I/O failure, a resource limit, or a broken internal invariant.

---

## 3 · Entry tokens

Only these byte sequences can begin a NextTeX construct:

```text
ntex-construct = at-construct | block-construct | raw

at-entry        = "@" at-keyword "("
block-entry     = "\" block-keyword "("
raw-entry       = "latex" ws* "{"

at-keyword      = "id" | "ref" | "import"
                | cite-command
                | "add" | "del" | "sub" | "note"

cite-command    = "cite" | "citep" | "citet" | "textcite" | "parencite"

block-keyword   = "figure" | "table"
```

A prefix is not enough. In particular:

- `@{` is ordinary LaTeX. It occurs 322 times in the measured corpus, all in tabular column specifications.
- `@` followed by a letter but not by a complete keyword and `(` is ordinary LaTeX.
- `@ref ` and `@ref{` are ordinary LaTeX because neither contains the entry token `@ref(`.
- `\figure` and `\table` without `(` are ordinary LaTeX control sequences.
- `latex` not followed by optional whitespace and `{` is ordinary text.

In 111 measured `.tex` files, `@{` was the most common `@` shape: 322 occurrences, compared with 143
occurrences of `@` followed by a letter. It does not collide with this grammar because an at-entry includes
the keyword and opening `(`.

None of the 111 files defines `\figure` or `\table`. This observation supports the block tokens for this
corpus but is not a claim about LaTeX generally. A source-level definition of either command before its first
NextTeX use is a name conflict attributed to the explicit NextTeX block. Detection outside modeled LaTeX is
advisory; an undetectable definition does not justify rewriting opaque bytes.

`latex {` is the weakest of the three entry tokens: it is a bare word rather than a sigil or a control
sequence, so a sentence could in principle open one. Measured across the same 111 files: **zero occurrences**
of `latex` followed by whitespace and `{` outside a command, and zero uses of `\latex` as a command. Safe on
this evidence, and the token most likely to need revisiting if a wider corpus contradicts it.

A complete entry token is recognized only in a recognition region. Section 8 defines regions in which the
same bytes remain ordinary text.

### Entry-token fixtures

Fixtures must pin these decisions:

1. `@{}` in a tabular column specification remains byte-identical.
2. `name@example.org`, `\@name`, `@ref ` and `@ref{a}` contain no construct.
3. `@ref(a)` in prose begins a reference.
4. `\figure(a) { ... }` begins a block, while `\figure { ... }` does not.
5. Every entry token inside each exclusion region in §8 remains ordinary bytes.

---

## 4 · Inline constructs

```text
at-construct = id-decl | reference | citation | import

id-decl      = "@id"     "(" ident ")"
reference    = "@ref"    "(" ident ")"
citation     = "@" cite-command "(" bibkey ( "," ws* bibkey )* ")"
import       = "@import" "(" string ")"

cite-command = "cite" | "citep" | "citet" | "textcite" | "parencite"

ident        = [A-Za-z] [A-Za-z0-9_:.-]*
bibkey       = [A-Za-z0-9] [A-Za-z0-9_:.+/-]*
```

The prefix before the first `:` in an identifier is not decoration: it is the class the reference demands,
per [`decisions/0003`](decisions/0003-the-prefix-is-the-demand.md). `@ref(fig:x)` requires a `Figure`. An
unmapped prefix, or none, demands nothing.

`@ref` emits the referenced label value only. The entity-kind word remains author text:

```latex
Definition~@ref(def:geakg)
Table~@ref(tab:roles)
Section~@ref(sec:results)
```

NextTeX does not generate `Definition`, `Table`, `Section`, `Appendix`, or a nonbreaking space. Generating
them would inject bytes absent from the source and would violate the binding erasure rule. v0.1 makes no
exception to that rule.

### Citations

**A citation construct is a LaTeX citation command written with `@`.** That is the whole rule, and it is
the same one `@ref` follows: NextTeX emits the command you named and knows nothing about the package that
defines it.

```
@cite(knuth1984)        emits  \cite{knuth1984}
@citep(knuth1984)       emits  \citep{knuth1984}
@citet(knuth1984)       emits  \citet{knuth1984}
@textcite(knuth1984)    emits  \textcite{knuth1984}
@parencite(knuth1984)   emits  \parencite{knuth1984}
```

A citation may name several keys, because 13% of measured citations do and one names seven:

```
@citep(knuth1984, lamport1994)   emits  \citep{knuth1984,lamport1994}
```

**Each key is checked separately.** One absent key is one diagnostic naming that key, not a diagnostic about
the construct.

Nothing else is emitted — no brackets, no `~`, no package. Checking the keys against the bibliography under
[`checking.md`](checking.md) §7 is the only thing the construct buys over writing the command directly.

The default set covers the LaTeX kernel (`\cite`), `natbib` (`\citep`, `\citet`) and `biblatex`
(`\textcite`, `\parencite`). A project using another command adds it in `nextex.toml`:

```toml
cite_commands = ["cite", "citep", "citet", "citealp"]
```

The list replaces the default rather than extending it, like every other map here.

#### Why a construct per command, and not one construct with a style field

A figure is one environment whose variants are fields, so `\figure` takes `src`, `width` and `caption`. A
citation is not one command with options: `\citep` and `\citet` are different commands producing different
output. Modelling them as one construct means inventing a vocabulary for the difference — `style=textual` —
and then mapping that vocabulary back onto a command, which cannot be done without knowing which package the
document loads. That is not knowable: `\usepackage` is absent in 17–50% of real projects that use a package,
because a class loads it (`tests/experiments/package-loading/`).

Naming the command directly removes the question. It also removes the vocabulary: an author who writes
`\citep` already knows what `@citep` is.

The earlier draft admitted `@cite(key, style=textual)` and never said what it emitted. Settled by the
director on 2026-08-29.

#### Migrating an existing project

Mechanical, and partial by design. Change `\citep{` to `@citep(` wherever you want the key checked. Every
citation you do not touch keeps working exactly as it does now, transported byte for byte and unchecked —
which is the same trade every other construct offers.

### `@id` attachment

`@id` attaches backwards to the nearest completed, attachable LaTeX construct. An attachable construct is:

- a sectioning command or other known command whose complete argument shape is provided by §8;
- a `\caption` command;
- a `\begin{...}` header, including its known optional and mandatory arguments;
- an environment just closed by `\end{...}`;
- a displayed equation just closed by `$$` or `\]`; or
- another construct explicitly listed as attachable by a package signature.

Between that construct and `@id`, the parser may skip only ASCII space, tab, carriage return and line feed.
The skipped region is bounded by both of these limits:

- no more than two line endings; and
- no more than 256 bytes.

The first limit reached ends the search. Comments, commands and non-whitespace bytes are not skipped. If no
attachable construct occurs within the bounded region, `@id` is an explicit but unattached annotation and
produces a hard error blamed on that annotation.

The two-line, 256-byte bound is a locality decision. The corpus establishes that same-line attachment is
insufficient: 442 of 479 measured labels follow their captions on the next line, while 9 share the caption
line. The evidence does not establish a larger useful distance. A corpus observation of correctly attached
labels separated by blank lines or by more than 256 whitespace bytes would settle whether either bound
should increase.

Examples:

```latex
\section{Introduction} @id(sec:intro)

\caption{A caption
  containing nested {groups}}
@id(fig:example)

\begin{theorem}[Bound]
  @id(thm:bound)
  ...
\end{theorem}
```

In the theorem example, `@id` attaches to the completed `\begin{theorem}[Bound]` header.

For an environment whose signature marks its body as display math — `equation`, `equation*`, `align`,
`align*`, `gather`, `gather*`, `multline` and their configured equivalents — the math exclusion region does
not begin at the end of the `\begin` token. It begins after:

1. the complete `\begin{name}` header and every argument selected by the environment signature;
2. at most two line endings and at most 256 intervening ASCII whitespace bytes; and
3. zero or more complete `@id` annotations attached to that header, including the bounded whitespace after
   each annotation.

A non-whitespace byte other than `@id(` closes this header slot and begins the math exclusion region at that
byte. Once the math exclusion has begun, entry tokens remain ordinary math bytes until the matching
`\end{name}`. A malformed `@id(` that starts within the header slot is a hard error and does not cause the
parser to search farther into the equation.

```latex
\begin{equation}
  @id(eq:energy)
  E = mc^2
\end{equation}
```

`$$...$$` and `\[...\]` have no header slot. They are declared after the closing delimiter, which is itself
a completed attachable construct:

```latex
\[
  E = mc^2
\]
@id(eq:energy)
```

An `@id` between the opening and closing delimiter is inside the math exclusion region and remains ordinary
bytes.

### Identifier scope and emission

An identifier is scoped to **one document root** — not one source file, and not the whole project. A root is
its root file plus the transitive closure of files reached by successful `@import` constructs, and its symbol
table is the merge of theirs.

Two declarations of one identifier in one root are a hard error blamed on the later explicit declaration.
Importing the same physical file twice does not redeclare its entities: a canonical path merges once per
root. Two different files declaring the same identifier do conflict when both belong to the same root.

Separate roots under one `nextex.toml` have separate symbol tables and may reuse an identifier, provided
neither imports the other's declaring file. This matches LaTeX's document-wide label namespace without
imposing a project-wide one on separately emitted documents.

`@id(sec:intro)` emits `\label{sec:intro}` at the annotation's source position, and nothing else — no
wrapper, no support command.

Before emission each root builds a **label inventory**. `\label` is a built-in known command with signature
`m`, so its mandatory argument is selectable under §8 even though its bytes are otherwise opaque. A label
enters the inventory only when it is tokenized as `\label` under default category codes, occurs outside every
§8 exclusion region, has one balanced braced argument, and that argument is an `ident` after trimming ASCII
whitespace.

The inventory is `Complete` or `Unavailable`. It becomes `Unavailable` when quarantine begins, a category-code
change prevents reliable tokenization, or a candidate argument cannot be bounded. None of those is an error
for an unannotated document.

When `Complete`, an `@id(x)` colliding with another `@id(x)` or with a source `\label{x}` is a hard error
blamed on the `@id`; nothing is emitted. The source `\label{x}` is never itself an error.

When `Unavailable`, an `@id` whose emitted label could not be checked is a hard error blamed on that
annotation, reported as *label inventory unavailable*, and no `\label` is emitted. This distinguishes lack of
evidence from absence, and stops a passing build from silently acquiring a duplicate label.

### Boundaries

`@id`, `@ref` and `@cite` end at the first `)` after their entry token. Their identifier, bibliography key and
field productions do not admit parentheses.

`@import` ends at the `)` immediately following its closing string quote. A `)` inside the string is data.

A malformed inline construct does not scan without bound:

- before a valid closing boundary is found, line end terminates the search for `@id`, `@ref` and `@cite`;
- `@import` may contain a line ending inside its quoted string only if the string syntax admits it; v0.1 does
  not, so line end terminates the search;
- the construct span ends immediately before that line ending;
- the explicit construct produces a hard error at its entry token;
- the line ending and following bytes are parsed normally.

### Inline fixtures

Fixtures must include:

1. `\caption{X}@id(x)` and a label on the immediately following line.
2. Attachment across two line endings and rejection across three.
3. Attachment after exactly 256 skipped bytes and rejection after 257.
4. A comment or non-whitespace byte between the target and `@id`, which prevents attachment.
5. `@ref(x)` followed immediately by punctuation.
6. A `)` inside an import string, which does not end the import.
7. An unterminated inline construct followed by a valid construct on the next line; the second remains
   recognizable.
8. Empty and non-ASCII identifiers, which are hard errors inside the explicit construct.

### Bibliography discovery and citation checking

Bibliography resources are discovered per document root, from configuration and from recognized LaTeX
declarations. Both contribute; neither overrides the other.

```toml
bibliographies = ["refs.bib", "sources/extra.bib"]
```

Paths are relative to the directory holding `nextex.toml`; an absolute path is invalid configuration.

The built-in signature set includes:

```text
\bibliography   m
\addbibresource o m
```

**These are known commands, so their arguments are selectable under §8** even though the bytes remain opaque
for transport. That is what makes citation checking possible at all.

- `\bibliography{a,b}` splits its argument on ASCII commas, trims ASCII whitespace, appends `.bib` where
  absent, and resolves each result relative to the file containing the command.
- `\addbibresource[...]{a.bib}` ignores the optional argument for discovery and resolves the mandatory one
  the same way.
- An argument containing a control sequence, inner brace nesting, a non-ASCII byte or an empty item is not
  expanded or guessed.

A command contributes only from a recognition region. A spelling inside a macro body, comment, verbatim
region, command argument, math region or quarantine contributes nothing.

```text
BibliographyState = Complete(KeySet) | Unavailable(Reason)
```

`Complete` requires every declared resource to be readable and every key boundary to parse. Keys come from
entries beginning `@`, an ASCII entry type, optional whitespace, `{` or `(`, then a `bibkey` terminated by
whitespace or a comma. `@comment`, `@preamble` and `@string` contribute none.

The state is `Unavailable` when no resource is declared, a path is dynamic or malformed, a resource is absent
or unreadable, a resource exceeds a configured limit, an entry boundary cannot be parsed, or quarantine could
hide a declaration. **Any failure makes the whole root `Unavailable` rather than yielding a partial key
set** — a partial set would produce false missing-key errors.

`@cite(k)` is a hard error only when the state is `Complete` and `k` is absent. Under `Unavailable` it
receives an advisory saying checking was unavailable, and the exit code does not change.


---

## 5 · Block constructs

```text
block-construct = block-keyword "(" ident ")" ws* block-body
block-body      = "{" field-list "}"

field-list      = ( ws* field ws* )*
field           = key ws* "=" ws* value
key             = [a-z] [a-z0-9_]*

value           = string | length | percentage | integer | braced | bare
```

Known fields determine the permitted value kind. A parser does not choose the kind by trying alternatives
until one succeeds.

### Value kinds

| Kind | Form | Ends at |
|---|---|---|
| `string` | `"` followed by bytes using `\"` and `\\` escapes | unescaped closing `"` |
| `length` | a decimal number plus `pt`, `mm`, `cm`, `in`, `em` or `ex`; **or** a TeX length such as `\linewidth`, with an optional coefficient before it | first byte outside that token |
| `percentage` | decimal number plus `%` | byte after `%` |
| `integer` | one or more ASCII digits | first non-digit byte |
| `bare` | bytes other than a line ending | immediately before the line ending |
| `braced` | balanced `{ ... }` | matching `}` |

`bare` exists for short scalar fields that explicitly declare it. It is not the value kind for `caption`,
`body` or trailing table content.

### Figure fields

```text
figure-fields:
  src       = string
  width     = length | percentage
  height    = length | percentage
  caption   = braced
```

### Table fields

```text
table-fields:
  caption   = braced
  body      = braced
  trailing  = braced
```

`trailing` contains content placed inside the emitted `table` environment after the table body and before
the environment terminator. It resolves the measured case in which `\vspace{2pt}` and a `\scriptsize`
footnote follow `tabular` but remain inside `table`:

```latex
\table(tab:roles) {
  caption = {RoleSchema instantiation for both case studies}
  body = {
    \begin{tabular}{@{}lcc@{}}
      ...
    \end{tabular}
  }
  trailing = {
    \vspace{2pt}
    {\scriptsize $^*$Edge counts depend on the generated topology.}
  }
}
```

A general positional-content form is not admitted in v0.1. The observed need is content trailing the main
table body, and a named field gives that content a deterministic emitted position. A future observation of
necessary content before, between or around several body regions would settle whether `trailing` is too
narrow.

`note` is not used as the field name because the measured bytes include spacing commands as well as prose,
and the grammar does not yet model the relation between `$^*$` and the trailing text.

### Captions

`caption` requires `braced`. In 479 measured captions:

- the median length was 230 characters;
- the longest was 1,160 characters;
- 111, or 23%, contained a newline; and
- 207 contained nested braces.

These measurements contradict the draft rule that made captions bare, to-end-of-line values.

### Column count

`columns` is not a v0.1 table field. The count is derived from the first top-level `tabular`, `tabularx`, or
other configured tabular environment in `body`.

The parser reads that environment's column-specification argument using its §8 signature. It then counts
columns from the specification, including repeated specifications and excluding `@{...}` insertions.
Structural checks must account for constructs such as `\multicolumn`; they must not compare a declared
number with another annotation.

If the environment or column specification cannot be bounded, the count is unavailable and the corresponding
check is advisory. It is not a hard error. A hard error may still arise from a malformed explicit `body`
field.

This decision removes duplicated information. The hand annotation restated
`\begin{tabular}{@{}lcccccc@{}}` as `columns = 7`, allowing the two annotations to disagree.

### Lengths

A `length` is emitted as written. A `percentage` becomes a fraction of a fixed reference, and the reference
is part of the field's meaning rather than something the author selects:

| Field | `80%` becomes |
|---|---|
| `width` | `0.80\linewidth` |
| `height` | `0.80\textheight` |

`\linewidth` is the only width correct both inside a single-column float and inside a spanning one; measured
by compiling, `\textwidth` overflows the first and `\columnwidth` under-fills the second. For height no
adaptive reference exists — TeX exposes no "space available in this float" — so `\textheight` is used, and
the asymmetry is what TeX offers rather than a choice.

**An author who needs a different reference names it, and that is why a `length` admits a TeX length:**

```
width = 80%                shorthand, 0.80\linewidth
width = 0.8\columnwidth     the reference named
width = \textwidth          no coefficient
width = 12cm                absolute
```

`\columnwidth` cannot be reached any other way. Inside a float that spans both columns, `\linewidth` is the
full page width, so a column-width image there has no percentage form. Restricting `length` to a number and
a unit made that unreachable without abandoning the block for plain LaTeX, and the restriction was written
without anyone asking whether it should hold.

A control word followed by `{` is a command taking an argument, not a length, and is rejected.

See [`decisions/0004`](decisions/0004-lengths-and-inclusion.md).

### Package requirements

`needs` is not a field. Writing it in a block is a malformed field like any other unknown key.

Packages are declared where LaTeX declares them: `\usepackage` in the preamble, written by the author,
transported byte for byte. Emitting one would inject bytes the erased build does not have, which contradicts
the binding no-injection rule and makes property B untestable rather than merely violated.

The alternative that would have kept the field — declare it, never emit it, check that the preamble already
loads it — was measured and rejected. Across 224 `.tex` files grouped by project, between 17% and 50% of
projects that use a package's commands never write its `\usepackage`: a journal class or another package
loads it, and the compiler reads neither `.cls` nor `.sty`. The check would have reported a missing package
in 9 of the 40 projects that use `booktabs`, each of which compiles.

Settled by the director on 2026-08-29 with that measurement in hand. Full record and the emission contract:
[`decisions/0001-typed-emission-and-no-injection.md`](decisions/0001-typed-emission-and-no-injection.md).
Reproduce the measurement with `tests/experiments/package-loading/`.

### Balanced-region scanning

For `braced`, block and raw boundaries, scanning uses a counter initialized to one at the opening `{`.

Under the default TeX category codes:

- an unescaped `{` increments the counter;
- an unescaped `}` decrements it;
- a `}` reducing the counter to zero is the boundary;
- an unescaped `%` begins a comment through the line ending, so braces in that comment do not affect the
  counter;
- `\{`, `\}` and `\%` are control symbols and do not open, close or start a comment.

For a run of consecutive backslashes immediately before `{`, `}` or `%`, the final byte is escaped only when
the run length is odd. This is decidable while scanning left to right by retaining the run length modulo two.

The corpus contains 120 `\%` sequences inside captions and no real comments inside those captions. Treating
every `%` as a comment would therefore truncate real caption content.

**A block body is scanned with one exception to that rule**, found while implementing it: `%` immediately
preceded by an ASCII digit is a percent sign, not a comment opener.

Without the exception, `width = 80%` cannot be parsed at all — the `%` opens a comment that swallows the
closing brace, and the block never ends. That is the same collision the tool baseline measured in plain
LaTeX, where `width=120%` fails with `File ended while scanning use of \Gin@ii` and the message names
nothing about percentages.

The exception needs one byte of context, so it stays a left-to-right rule. It applies **only inside a
NextTeX block body**. Transported LaTeX keeps TeX's rule, where a comment means what TeX says it means.

The alternative was to require `80\%` inside blocks. It was rejected because the escape would appear in
NextTeX's own syntax purely to work around a rule NextTeX controls.

A top-level category-code change makes these default rules unreliable and invokes quarantine under §8.

### Block boundaries

A block ends at the `}` that reduces its block-body nesting counter to zero. It may span the rest of the
file; no arbitrary byte lookahead limit applies because each byte is consumed once. The nesting resource
limit still applies.

An unmatched block opening consumes through end of file and produces a hard error blamed on the explicit
block.

A field ends at the boundary specified by its declared value kind. The next non-whitespace byte must either
start another field or be the block-closing `}`.

### Block fixtures

Fixtures must include:

1. A multi-line caption containing nested braces.
2. A caption containing `\%`, `{10\%}`, a real comment, `\{` and `\}`.
3. Odd and even runs of backslashes before `%`, `{` and `}`.
4. A `trailing` field containing both a spacing command and a braced font-size group.
5. `@{}` inside a tabular column specification.
6. A derived column count with `@{...}`, `*{n}{...}` and `\multicolumn`.
7. An unknown tabular environment, for which column checking becomes advisory.
8. `caption = unbraced text`, `columns = 7` and `needs = booktabs`, each rejected as a malformed explicit
   block.
9. An unmatched block opening whose boundary is end of file.

---

## 6 · Revision constructs

```text
revision = add | del | sub | note

add      = "@add"  "(" ident ")" ws* revision-content
del      = "@del"  "(" ident ")" ws* revision-content
sub      = "@sub"  "(" ident ")" ws* substitution-content
note     = "@note" "(" ident "," ws* "on" ws* "=" ws* ident ")" ws*
           revision-content

revision-content     = braced
substitution-content = "{" substitution-left "->" substitution-right "}"
```

Revision constructs are inline constructs. They may appear between words, punctuation or other prose:

```latex
The result is @add(change:qualified) {statistically significant} under both tests.
```

They may also span lines. The measured revision convention appeared 179 times: 175 uses were phrases inside
sentences, 4 were multi-line, and the longest contained 2,040 characters. The draft's block-oriented
implication was therefore incorrect.

### Nested constructs

Recognition is enabled inside revision content, subject to §8. Thus references, citations, revision
constructs and other eligible constructs may be nested:

```latex
@add(change:source) {As reported by @cite(knuth1984), see @ref(tab:result).}
```

This decision follows the 33 measured revisions containing `\ref`, `\cite` or `\label`. Treating revision
content as opaque would prevent those dependencies from being represented.

Nested revisions must be properly contained. Crossing intervals are impossible in the brace grammar, and
**the sidecar cannot reintroduce them**: it is keyed by identifier and carries no spans, settled by the
director on 2026-08-29. So non-overlap is not a rule the parser enforces, it is a property the
representation has. There is no fixed nesting-depth lookahead; a counter and stack are updated left to
right, subject to the configured nesting resource limit.

The content of a revision is in the file; the author, the timestamp and the conversation are in the sidecar,
keyed by revision identifier. Acceptance is not a stored state — it rewrites the source. See
[`revisions.md`](revisions.md).

### Substitution separator

For `@sub`, the separator is the first `->` at brace depth one that is outside all §8 exclusion regions and
outside a nested NextTeX construct. A second such separator before the outer closing brace is a hard error.
An arrow inside a nested brace group, command argument, comment, math region, raw escape or nested construct
is content.

This rule is decidable left to right using the current brace depth and region stack.

### Revision boundaries

`@add`, `@del` and `@note` end at the `}` matching their content's opening `{`.

`@sub` ends at the matching outer `}` after exactly one top-level separator.

A missing content opening brace ends the malformed construct at the first non-whitespace byte after its
header. An unmatched content brace ends at end of file. Both are hard errors blamed on the explicit
revision.

### Revision fixtures

Fixtures must include:

1. A revision between two words and one followed immediately by punctuation.
2. A multi-line revision longer than 2,040 bytes.
3. Nested `@ref`, `@cite`, `@add` and `@del`.
4. A nested construct inside a command argument in the revision; §8 keeps it ordinary text.
5. A substitution with arrows at nested brace depth and exactly one at depth one.
6. Zero and two depth-one substitution arrows, both hard errors.
7. An unmatched revision brace ending at end of file.
8. Two properly nested revisions. The crossing case is not a fixture: the sidecar carries no spans, so
   there is nothing that could express it.

---

## 7 · The raw escape

```text
raw = "latex" ws* braced
```

```latex
latex {
  \begin{tikzpicture}
    \draw (0,0) -- (2,1);
  \end{tikzpicture}
}
```

The raw escape is **also the escape for a literal entry token in prose**:

```latex
The token is latex {@ref(example)}.
```

It emits only the bytes inside its outer braces — the `latex`, the whitespace and the outer brace pair are
erased — so the emitted text is `The token is @ref(example).`

This adds no new interpretation: `latex` followed by optional whitespace and `{` was already the raw
construct of §3, so no existing valid `.tex` changes meaning. Nested braces stay part of the body; only the
outer pair is erased.

Raw content is transported. No NextTeX entry token is recognized inside it. All raw content is unchecked
for the coverage figure reported by `nextex check`.

The raw escape ends at the `}` matching its opening `{`, using the balanced-region rules in §5. An unmatched
opening brace ends at end of file and produces a hard error blamed on the explicit raw escape.

### Raw fixtures

Fixtures must include:

1. Every entry token inside a raw escape, with none recognized.
2. Nested braces, escaped braces, `\%` and a real comment.
3. An unmatched raw opening brace ending at end of file.
4. `latex` not followed by optional whitespace and `{`, which remains ordinary LaTeX text.

---

## 8 · Where constructs are not recognized

This section separates a language from text substitution. In each region below, `@ref(` and every other
entry token are ordinary bytes.

| Region | Begins at | Ends at |
|---|---|---|
| Comment | unescaped `%` under current default category codes | line ending or end of file |
| Inline math | unescaped `$` not followed by `$` | next corresponding unescaped `$` |
| Display math | `$$` or `\[`; for a display-math environment, the first byte after the header slot in §4 | corresponding `$$`, `\]` or matching `\end{name}` |
| Verbatim command | a known verbatim command and its delimiter byte | next occurrence of that delimiter |
| Verbatim environment | a configured verbatim `\begin{name}` | its line-exact `\end{name}` |
| Listing | `\begin{lstlisting}` or a configured listing name | its line-exact terminator |
| Internal-macro region | `\makeatletter` | `\makeatother` |
| Raw escape | complete `latex` entry through its opening `{` | matching `}` |
| Command argument | an argument start selected by a known signature | that argument's specified boundary |
| Quarantine | a hazard listed below | end of file |

The default verbatim environment set is `verbatim`, `verbatimtab`, `Verbatim`, `listing` and `lstlisting`,
extended by `nextex.toml`.

The verbatim **command** set is `\verb` and `\verb*` from the LaTeX kernel, `\lstinline` from `listings`, and
`\mint` and `\mintinline` from `minted`. The last three are transcribed from unified-latex, which marks them
`argumentParser` rather than giving them a signature — the database itself records that their arguments
cannot be described by one, which is the same reason they are listed here. `fancyvrb`'s `\Verb` is not in the
set because no source for it was read; adding it requires reading one.

Each takes its delimiter after any arguments that precede it, and ends at that byte's next occurrence.
`\lstinline` and `\mintinline` may also take a braced form, which is an ordinary balanced group.

Two boundary rules, both compiled rather than recalled:

- **Spaces and tabs before the delimiter are skipped.** TeX absorbs the spaces after a control word.
  `\verb xCODEx` typesets `CODE`, not `xCODEx`, so the delimiter is `x` and not the space.
- **A line ending before the delimiter opens no region at all.** `\verb` followed by a newline typesets the
  next line's `|CODE|` as ordinary text. Recognition continues; it does not quarantine. Quarantining would
  cost the rest of the file over bytes TeX itself ignores, and `\lstinline` named in prose is a sentence
  people write.

Fixtures `exclusions/15` and `exclusions/16`.

An unterminated excluded region whose boundary cannot safely be recovered enters quarantine rather than
resuming recognition at a guessed byte.

### Command argument regions

#### A signature is a claim the call can refute

A command name does not determine a signature. Of the 130 built-in signatures, **15 carry a different one
under another package**: `\definecolor` is `m m m` under `color` and `o m m m` under `xcolor`; `beamer` adds a
`<overlay>` argument to twelve commands including `\section`, `\item` and `\label`; `cleveref` redefines
`\ref` from `s m` to `m`. Measured against unified-latex, package by package.

So the rule is not "look up the signature and follow it". It is:

1. Look up the signature and try to follow it.
2. If a mandatory argument is required where the bytes hold `[`, `<`, `(` or `*`, the signature does not
   describe this call. **Discard it** and fall through to the unknown-command rule below.

The fall-through is what makes package ambiguity safe without reading the preamble. Every one of the 15
variants differs by adding an argument at the front, so the mismatch is detected at the first argument and
the conservative path — exclude the adjacent groups — is taken instead.

Trusting the signature anyway is the failure this prevents, and it is not hypothetical: with `m m m` applied
to `\definecolor[named]{...}{...}{...}`, the `m` consumed the single byte `[`, recognition resumed **inside**
`named`, and all three arguments were opened to construct recognition.

Reading `\usepackage` from the preamble to pick the right variant would be an improvement in precision. It is
not required for safety, and it is not in v0.1.

#### Where signatures come from

Command shapes are not inferred from the corpus. They come from declarative specifications:

1. an `xparse` argument specification present in the source or project configuration;
2. the unified-latex CTAN package signature database under
   `packages/unified-latex-ctan/package/*/provides.ts`; and
3. built-in signatures transcribed from the LaTeX specifications.

For example, the signature `o m m m` selects one optional argument followed by three mandatory arguments.
An `m` argument begins at its mandatory argument token and, when braced, ends at its matching `}`. An `o`
argument begins at `[` and ends at its balanced matching `]`. Other `xparse` argument kinds use their
specified delimiter and boundary.

Only arguments selected by the signature are excluded. Whitespace permitted by that signature is consumed
while advancing to the next argument. This requires no unbounded lookahead: each signature is finite and
each delimited argument is scanned once. A configured maximum of 64 arguments per signature bounds
malicious specifications; exceeding it is a resource-limit error.

For a command absent from all signature sources:

- its control-sequence token is transported as opaque;
- no following group is asserted to be its argument;
- balanced groups immediately following it remain ordinary LaTeX regions and do not produce hard errors;
- entry-token-shaped text within those groups may be reported only as an advisory ambiguity and is not
  recognized as a NextTeX construct.

The last rule prevents accidental interpretation inside an unknown macro body without inventing that
command's shape. Recognition resumes after the balanced adjacent group. If such a group cannot be bounded,
parsing enters quarantine.

Whether all immediately adjacent groups of an unknown command should be excluded remains open. v0.1
excludes a consecutive run of at most 16 balanced `{...}` or `[...]` groups, separated only by whitespace.
After 16 groups, parsing enters quarantine rather than guessing that a seventeenth group is prose. A real
command absent from the databases with more than 16 arguments, or a documented collision after a shorter
run, would settle a change to this bound.

### Environment headers and bodies

A known environment's `\begin{name}` arguments are selected from its signature and excluded as command
arguments. Its body is not excluded merely because it belongs to an environment. Separate rows in the table
above, such as verbatim and listing, may exclude it.

This distinction permits prose and nested NextTeX constructs in ordinary environment bodies while keeping
an optional title such as

```latex
\begin{definition}[Generative Executable Algorithm Knowledge Graph]
```

opaque unless the environment signature declares that optional argument.

For an environment marked as display math by its signature, the body exclusion begins only after the equation
header slot specified in §4. This is the sole exception to immediate body exclusion: it admits `@id` in that
bounded slot and no other construct in the remaining math body.

### Quarantine

```text
ParseConfidence = Structured | OpaqueBalanced | OpaqueToEof
```

`OpaqueToEof` begins at:

- an unterminated verbatim command or verbatim environment;
- an unmatched `\makeatletter`;
- a top-level `\catcode` assignment whose effect cannot be bounded;
- an unterminated unknown-command group;
- an exclusion-region opener whose end cannot be located; or
- a configured resource bound that requires preservation rather than rejection.

Quarantine is monotonic within the file: after `OpaqueToEof` begins, no later NextTeX construct is
recognized. The remaining bytes are transported.

### Exclusion fixtures

Each row in the exclusion table requires a fixture containing every entry-token family and an exact expected
end byte. Additional fixtures must include:

1. `\%` outside a comment and `%` beginning a comment.
2. An entry token immediately after the closing math delimiter.
3. An entry token equal to or containing a verbatim delimiter.
4. Mixed-case and configured verbatim environment names.
5. A known `o m m m` command with entry tokens in every argument and one immediately after the last.
6. An unknown command followed by 1, 16 and 17 adjacent groups.
7. A known environment optional title containing `@ref(` and a body containing a valid `@ref`.
8. An unmatched opener for every quarantine-producing region, followed by an apparent construct that remains
   opaque.

---

## 9 · Entity coverage and v0.1 limits

### Admitted entities

v0.1 admits:

- figures and tables through typed blocks;
- sections, equations, definitions, theorems, algorithms, appendices represented by sectioning commands, and
  other known LaTeX constructs through `@id`;
- a subfigure panel by attaching `@id` to its `subfigure` environment header;
- an algorithm as an environment entity by attaching `@id` to its environment header.

Algorithms are explicitly included at the light-annotation level because the measured corpus contains 49
algorithm environments, more than its theorem environments. A typed algorithm block is not added because
no measured typed check requires one.

Subfigure panels are included at the light level because 34 `subfigure` environments occur in the corpus.
No typed panel fields are specified because the available observations establish naming demand but not a
stable field set.

An appendix remains a section entity. The source must retain the manual kind word in prose. v0.1 does not
add a separate appendix block because `@id` already provides naming and no appendix-specific checked field
has been demonstrated.

### Not admitted as separate entities

v0.1 does not provide separate identities for:

- a table row;
- a table cell;
- a footnote marker inside a cell;
- the relation between a marker and trailing table content;
- an environment's optional title; or
- a numbered line inside an algorithm.

Rows and cells are governed by TeX alignment, macro expansion, `\multicolumn`, and package-specific commands.
The hand annotation demonstrates the need to relate a cell marker to trailing content but does not establish
a syntax or reliable byte boundary for either entity.

Algorithm lines are common — 237 `\STATE` lines in 49 environments — but the available evidence does not
state which algorithm packages and command signatures produced them, whether line numbers correspond
one-to-one with commands, or how continuation lines behave. Frequency establishes priority, not a grammar.

These decisions would be settled by:

- annotated examples from the relevant table and algorithm packages;
- the package signatures for row, cell and line commands;
- cases involving `\multicolumn`, nested tabular content, continued algorithm lines and suppressed line
  numbers; and
- a required check or reference operation that cannot be expressed by attaching `@id` to an existing
  bounded construct.

The optional environment title remains data inside a known argument region. An observation requiring a
reference specifically to that title, rather than to its environment, would justify a new construct.

---

## 10 · Why no parser generator

The parser is hand-written recursive descent with a hand-written lexer.

**The difficult boundary rules are not context-free.** `\verb` uses the next byte as its delimiter.
Verbatim environments consume raw lines. Category-code assignments change byte roles. `\makeatletter`
changes control-sequence tokenization. In a parser generator these rules would still require lexer modes and
procedural predicates.

**Transport requires original bytes.** The syntax tree stores spans into an immutable byte buffer.
Whitespace, comments and opaque regions cannot be decoded, normalized or reconstructed.

**Recovery is specified behavior.** Quarantine downgrades a region to opaque and preserves it. It is not a
generator's default error-recovery policy.

ANTLR has no official Rust target. A PEG could express the native syntax but would leave the LaTeX boundary
scanner hand-written, producing two parsing systems without removing the difficult part.

---

## 11 · Decisions, corrections and remaining observations

### Changes from the draft

| Draft rule | Corrected rule | Evidence or binding reason |
|---|---|---|
| `@id` required same-line adjacency | Skips bounded whitespace across up to two line endings and 256 bytes | 442 of 479 labels followed their caption on the next line; only 9 shared its line |
| Caption defaulted to a bare end-of-line value | `caption` requires balanced braces | 23% of captions contained a newline; 207 contained nested braces |
| `%` handling did not expressly exempt `\%` | Odd-backslash `\%` is not a comment opener | 120 escaped percent signs and zero real comments occurred inside measured captions |
| Revision nesting was unspecified | Revisions work mid-sentence and recognize nested constructs | 175 of 179 uses were inline; 33 contained a reference, citation or label |
| Entry-token discussion omitted `@{` | `@{` is explicitly ordinary LaTeX | 322 occurrences, all in tabular column specifications |
| Block-token safety lacked the larger measurement | The corpus boundary is stated | No file among 111 defined `\figure` or `\table` |
| Raw-escape token safety was unstated | `latex {` measured and recorded as the weakest token | Zero occurrences of `latex` + whitespace + `{` in prose across 111 files |
| Tables had nowhere for internal trailing content | Tables have a balanced `trailing` field emitted inside the environment | The hand annotation displaced a spacing command and table note outside their table |
| Reference kind words were unresolved | Kind words remain manual | Generating them would violate erasure and no-injection |
| `columns` was declared | Column count is read from the tabular column specification | The hand annotation duplicated information already present in `@{}lcccccc@{}` |
| Argument-position detection was left for corpus validation | Command arguments are selected from `xparse` and unified-latex signatures | LaTeX already provides declarative command-shape specifications |
| Entity coverage was implicit | v0.1 admissions and exclusions are explicit | The hand annotation and counts for algorithms, lines and subfigures exposed the missing cases |
| `needs` was admitted while its effect remained unresolved | `needs` is not a field | Emitting `\usepackage` injects bytes; the non-emitting check was measured wrong in 17–50% of real projects. Settled by the director, `decisions/0001` |

### Open decisions

The following decisions remain open because the available evidence does not select one option:

1. **`@id` whitespace bound.** v0.1 uses two line endings and 256 bytes. Correct labels beyond either bound
   would justify increasing it; a collision caused within the current bound would justify reducing it.
2. **Unknown-command adjacent groups.** v0.1 excludes at most 16. Package examples with more arguments, or
   prose collisions after fewer groups, would settle a different bound.
3. **Table marker relations.** Options include typed row, cell and note-reference constructs. Annotated
   examples covering alignment and `\multicolumn` would determine their boundaries.
4. **Algorithm lines.** Options include attaching `@id` to a package command or adding a typed line
   construct. Package-specific examples relating source commands to rendered line numbers would select one.
5. **Optional environment titles.** Options are to keep them opaque or expose a typed field. A required
   title-specific check or reference would justify the latter.
6. **Typed algorithm and panel blocks.** Light annotations admit both. A demonstrated typed check with a
   stable field set would justify new blocks.
7. ~~**What a percentage lowers to.**~~ Settled 2026-08-29: `width` against `\linewidth`, `height` against
   `\textheight`, no keyword. See §5 and `decisions/0004`.
8. ~~**Whether `@import` lowers to `\input` or `\include`.**~~ Settled 2026-08-29: `\input`, and
   `\include` was never available — it cannot be nested and `@import` nests. See `decisions/0004`.
11. **Whether `keepaspectratio` is emitted when a block gives both `width` and `height`.** Of 243 measured
    `\includegraphics` calls, 17 give both and 15 of those also give `keepaspectratio`, so the intent is
    almost always a bounding box rather than a stretch. Following it would be a second exception to
    no-injection and needs its own decision record.
9. ~~**What `@cite` emits.**~~ Settled 2026-08-29 by the director: a citation construct is a LaTeX
   citation command written with `@`, and it emits that command. No style vocabulary, no package
   inference, no fields. See §4.
10. ~~**Package requirements.**~~ Settled 2026-08-29: `needs` is not a field, packages are source-authored.
   See `decisions/0001-typed-emission-and-no-injection.md`.
