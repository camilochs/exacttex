# Revisions

A change model inside the file, rather than an editor feature bolted on beside it.

Word and Google Docs both track changes, and both keep them in a format no human reads and no version
control can diff. LaTeX has nothing at all, so every tool that wants the feature — `changes.sty`,
`latexdiff`, a journal's own portal — invents a layer of its own and none of them agree.

Here the change is part of the document, and this document says what it means.

---

## 1 · Where a change lives

Split, and the split is the design:

| | Lives in | Why |
|---|---|---|
| The text of the change | the `.ntex` file | It is content. It belongs in the diff, in the commit, in the branch. |
| Who, when, and the conversation | `paper.ntexrev`, beside it | It changes many times per review and would bury the text's own diff. |

The consequence that matters: **losing the sidecar loses attribution, never text.** A `.ntex` whose
`.ntexrev` was deleted still renders every view correctly; it just cannot say who proposed what. The
reverse is not true and cannot happen, because the sidecar holds no content.

The second consequence is structural. The sidecar carries **no spans** — it is keyed by identifier alone. So
a sidecar cannot describe a change that crosses another one, and the non-overlap rule is not enforced but
inherited: brace nesting makes crossing intervals unrepresentable. The rule costs nothing to hold because
there is no way to write a violation.

---

## 2 · The four constructs

The grammar is in [`grammar.md`](grammar.md) §6. Their meanings:

| Construct | Means | In `--original` | In `--final` |
|---|---|---|---|
| `@add(c) {text}` | text proposed for insertion | nothing | `text` |
| `@del(c) {text}` | text proposed for removal | `text` | nothing |
| `@sub(c) {old -> new}` | one replaced by the other | `old` | `new` |
| `@note(c, on = c2) {text}` | a comment about `c2` | nothing | nothing |

`@note` never contributes bytes to any built document. It is a remark about another revision, and it exists
because a review is a conversation and the conversation should live where the change does.

Recognition is enabled inside revision content, so a proposed sentence may carry `@cite` and `@ref` and be
checked like any other prose. That follows the 33 measured revisions in the corpus containing `\ref`,
`\cite` or `\label`; treating the content as opaque would have made those dependencies invisible.

---

## 3 · The three views

```
                       paper.ntex
                            |
        +-------------------+-------------------+
        |                   |                   |
   --original           --marked             --final
   before any        every change         every change
    change            visible              applied
```

- **`--original`** — the document as it stood before any revision. Every `@add` and `@note` contributes
  nothing, every `@del` contributes its text, every `@sub` contributes its left side.
- **`--final`** — the document with every revision applied. The mirror of the above.
- **`--marked`** — one document showing every change, coloured and struck through, for reading rather than
  submitting.

**Each view is one document.** There is no status, flag, or configuration that makes `--final` render two
ways. This is deliberate and it is why accepting is a source rewrite rather than a stored state: a stored
`accepted`/`pending` distinction would immediately raise "does `--final` include pending?", and either
answer gives the same file two final forms.

`--original` and `--final` are ordinary builds and property B holds for both. `--marked` is not — see §6.

---

## 4 · Accepting and rejecting

```sh
nextex revise --accept change:qualified
nextex revise --reject change:qualified
nextex revise --accept-all
```

Accepting **rewrites the source**. The construct disappears and the text it argued for stays:

```latex
before:  The result is @add(change:qualified) {statistically significant} under both tests.
accept:  The result is statistically significant under both tests.
reject:  The result is under both tests.
```

That is what Word does when you accept, and it is the honest model: a change that has been accepted is no
longer a change, it is the document.

**Substitution is atomic.** `@sub(c) {old -> new}` becomes `new` or `old`, never both and never neither, and
never a file containing half of one. The whole file is written once, to a temporary path in the same
directory, then renamed over the original. A crash leaves the source untouched.

**Rejection is recorded, not discarded.** This is the one place the model beats Word. Rejecting appends the
removed text to the sidecar's history, so a review that threw away a paragraph can still produce it six
months later:

```toml
[[history]]
id         = "change:qualified"
kind       = "add"
resolution = "rejected"
by         = "camilochs"
at         = "2026-08-29T11:04:00Z"
removed    = "statistically significant"
```

Word keeps the accepted text and loses the rejected text. Here both survive, one in the document and one in
the record.

---

## 5 · The sidecar

`paper.ntexrev`, TOML, beside the `.ntex` it belongs to.

```toml
version = 1
document = "paper.ntex"

[[revision]]
id     = "change:qualified"
kind   = "add"
author = "reviewer-2"
at     = "2026-08-28T09:12:00Z"
message = "The unqualified claim overstates the CI."

[[revision]]
id     = "note:ci"
kind   = "note"
on     = "change:qualified"
author = "camilochs"
at     = "2026-08-28T14:30:00Z"
message = "Agreed. Rewriting with the interval."
```

| Field | Required | Meaning |
|---|---|---|
| `id` | yes | The identifier in the construct. Unique within the document root. |
| `kind` | yes | `add`, `del`, `sub` or `note`. Must match the construct. |
| `author` | yes | Free text. NextTeX does not know what an account is. |
| `at` | yes | RFC 3339 timestamp. |
| `message` | no | Why. |
| `on` | notes only | The `id` this note is about. |

### Identity is by `id`, and it is one-to-one

Three rules make the mapping deterministic, which is what the issue's exit criterion asks for:

1. Two `@add(c1)` in one document root is `NT1001`, the same duplicate-identifier error as two `@id`. So one
   identifier names at most one construct.
2. Two `[[revision]]` blocks with the same `id` is `NT1010`. So one identifier names at most one record.
3. `kind` in the record must match the construct's keyword, `NT1011`. A record that says `add` against a
   `@del` is a corrupted pairing rather than a rename.
4. The sidecar's `document` must name the file it sits beside, `NT1013`. One naming a different file is
   paired with the wrong source, and every record in it would be judged against constructs it was never
   about. The same code covers a sidecar that cannot be read at all.

### Orphans, in the two directions that are not the same

**A record with no construct** — someone deleted the `@add` by hand and left the record. This is `NT1012`, a
hard error, and `nextex revise --prune` fixes it by moving the record to `history`.

**A construct with no record** — the change is in the document, its attribution is not. This is an
**advisory**, never an error. The document builds, every view is correct, and the only loss is that NextTeX
cannot say who proposed it. That is exactly the cost §1 chose to accept, so failing on it would contradict
the design.

The asymmetry is the point. The file is authoritative for content; the sidecar is authoritative for
attribution. A missing sidecar is a smaller problem than a missing document, and the errors say so.

Note that `NT1012` is a hard error with no explicit construct behind it, which the invariant in
[`AGENTS.md`](../AGENTS.md) §4 otherwise forbids. The scope of that invariant is *ordinary LaTeX*: a
renamed `.tex` must check clean, and it does — a document with no `.ntexrev` has no records to orphan. A
`.ntexrev` is NextTeX's own artefact, and an inconsistency inside it is ours to report.

---

## 6 · The marked view injects, and it is the only thing that does

`--marked` emits colour and strikethrough, so it emits commands no field asked for and loads the packages
that provide them. That is injection, forbidden everywhere else by
[`decisions/0001`](decisions/0001-typed-emission-and-no-injection.md).

It is permitted here, bounded by three conditions:

1. **`--marked` output is never the artefact of record.** It is written to a distinct filename and is not
   what a journal receives.
2. **Property B does not cover it.** The property says annotating must not change the rendered page. The
   entire purpose of this view is to change the rendered page, so applying the property to it would be a
   category error rather than a violation.
3. **The normal build is unaffected.** `nextex build` on the same source emits the same bytes whether or not
   `--marked` was ever run.

Recorded as [`decisions/0002`](decisions/0002-the-marked-view-may-inject.md), because decision 0001 says a
second exception to no-injection needs its own record.

---

## 7 · What this does not do

- **No merge.** Two people editing the same `.ntex` resolve it in git, like any other file. NextTeX has no
  opinion about branches.
- **No identity.** `author` is a string. There is no account, no signature, no verification that the person
  named is the person who typed.
- **No UI.** The constructs and the sidecar exist so that a UI can be built on them. That is deliberate
  sequencing, not an omission: a change model designed around a particular editor's affordances stops being
  a document format.
- **No automatic identifiers.** `@add(c1)` is named by whoever writes it, like every other identifier.
  Generating them is a job for the tool that inserts revisions, not for the language.
