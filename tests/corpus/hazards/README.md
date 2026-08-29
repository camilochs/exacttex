# Hazard fixtures

One per row of the parser hazard table in [`ROADMAP.md`](../../../ROADMAP.md). Each carries the
**falsifying observation** from that table — the thing that, if seen, means the handling is wrong.

They are written here rather than found in a corpus, because a corpus contains what its authors happened to
write, and these are the cases nobody writes on purpose.

## Not handled yet

The shallow LaTeX parser is [#21](https://github.com/camilochs/exacttex/issues/21) and does not exist. Every
fixture below is specified and unsatisfied, which is the point of writing them first:

| Fixture | Specified behaviour |
|---|---|
| `01-verb-without-a-terminator` | `OpaqueToEof`, no invented terminator |
| `02-verbatim-holding-every-marker` | opaque body, recognition resumes after |
| `03-catcode-at-top-level` | `OpaqueToEof` |
| `04-makeatletter-unmatched` | `OpaqueToEof` |
| `05-newcommand-body-with-a-marker` | opaque body; name and arity recorded |
| `06-csname-generated-name` | `OpaqueBalanced`; no inferred name |
| `07-conditional-both-branches` | both branches opaque, condition unevaluated |
| `08-input-is-an-edge-not-a-splice` | an opaque project edge |
| `09-newenvironment-shell-only` | shell located, bodies opaque |
| `10-verbatim-name-that-is-not-configured` | ordinary — no guessing |

The last one is the only one that asserts something must **not** happen. Guessing that an unknown
environment is verbatim would silently stop checking content the author expects checked, and the remedy is
`xtex.toml` rather than inference.
