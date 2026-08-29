# Key reader vs. the `biblatex` crate

Issue #12 named `biblatex` as the bibliography parser. This measures whether adopting it would help.

Both readers extract the citation keys of a `.bib` file. Only the key set matters: it is the only thing
citation checking asks for.

## Run

```sh
cargo run --release -- $(find <corpus> -name '*.bib')
```

Paths with spaces need quoting; the author's corpus has five.

## Result, 2026-08-29

37 `.bib` files from the author's projects.

| Outcome | Files |
|---|---|
| Identical key sets | 36 |
| `biblatex` failed to parse the file at all | 1 |

The failure is `irace-package.bib`, shipped inside the `irace` R package. It opens with an `@preamble`
concatenating brace groups with `#`; the crate expects a quotation mark and stops at byte 12. BibTeX accepts
the file, and the hand reader finds its 25 keys.

Under the rule in [`docs/checking.md`](../../../docs/checking.md) §2, a parse failure makes the whole
bibliography `Unavailable`, which silences citation checking for that document. So on this corpus the crate
would cost coverage on one file and match on the rest.

That is the whole basis of the decision, and it is one run over one corpus.
