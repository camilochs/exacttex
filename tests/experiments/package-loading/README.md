# Is a package's use visible in the source?

Decides the `needs` field. The proposal was: a block declares the packages its body requires, the compiler
checks the preamble already loads them, and nothing is emitted. That keeps the syntax and breaks no
invariant — if the check can be trusted.

## Run

```sh
python3 measure.py ~/Workspace
```

`.tex` files are grouped by directory, and a package counts as explicitly loaded if any file in that group
carries a `\usepackage` or `\RequirePackage` naming it.

## Result, 2026-08-29

224 `.tex` files from the author's workspace.

| Package | Projects using it | With no explicit `\usepackage` |
|---|---|---|
| amsmath | 18 | 9 (50%) |
| hyperref | 35 | 16 (46%) |
| xcolor | 42 | 17 (40%) |
| graphicx | 30 | 10 (33%) |
| booktabs | 40 | 9 (22%) |
| subcaption | 16 | 3 (19%) |
| multirow | 6 | 1 (17%) |

Between a sixth and a half of real projects use a package's commands and never write its `\usepackage`. A
journal class or another package loads it, and the compiler reads neither `.cls` nor `.sty`.

So the check would report a missing package in 9 of the 40 projects that use `booktabs` — every one of them
a document that compiles. A check wrong that often trains its reader to ignore it.

`needs` was removed on this evidence. See
[`docs/decisions/0001`](../../../docs/decisions/0001-typed-emission-and-no-injection.md).

## What would change the answer

Reading `.cls` and `.sty` to follow indirect loading, which is Phase 4 work. Re-run this then: the numbers
should fall, and the case for the field could be made again on new evidence.

## Limits of this measurement

One author's corpus. Grouping by directory approximates a project and will merge two projects that share a
folder. The probe commands imply their package but do not exhaust it — a project using `booktabs` only
through a template macro is not counted as using it.
