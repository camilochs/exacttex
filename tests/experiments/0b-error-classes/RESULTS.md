# Stage 0b — what existing tools say about each Class-A defect

Measured 2026-09-01 on macOS (Darwin 25.4.0).
Tools: Tectonic 0.16.9 · ChkTeX v1.7.9 - Copyright 1995-96 Jens T. Berger Thielemann. · texlab 5.26.0 · chklref 3.1.2 · xtex-cli at commit 097eb88.
Protocol: for each defect class, the minimal LaTeX carrying it (`cases/<name>/main.tex`)
ran through each tool; the annotated twin (`main.xtex`) ran through `xtex check`.
Runner: `run.sh` (tectonic/chktex/chklref) and `texlab-probe.mjs` (LSP diagnostics).

**Prediction, registered in the plan before running** (exacttex-plan-v5 §11, 0b):
only the wrong-entity-class defect goes undetected by everything; the missing
figure file is already a hard error from TeX itself; the rest are soft warnings.
Failure criterion: four or more already detected as hard error.

## Results

| Defect | tectonic (build) | chktex | chklref | texlab (editor) | xtex check |
|---|---|---|---|---|---|
| Broken reference | exit 0, PDF ships "??", warning only in the log | silent | silent (reports an *unused* label instead) | ERROR squiggle "Undefined reference" | **hard error XT1003**, names the entity, suggests the near-miss |
| Missing citation | exit 0, "??" | style note only (`~` spacing) | silent | ERROR "Undefined reference" | **hard error XT1005**, names the key |
| Duplicate identifier | exit 0, silent to terminal | silent | silent | ERROR "Duplicate label" ×2 | **hard error XT1001**, points at the first declaration |
| Wrong entity class (prose says Figure, target is a table) | **undetected** | **undetected** | **undetected** | **undetected** | **hard error XT1004**: "requires figure, but its target is table", declaration linked |
| Missing figure file | **hard error** (TeX itself) | silent | silent | silent | **hard error XT1006**, before TeX runs |
| Invalid unit | **hard error** (TeX: "Illegal unit") | silent | silent | silent | **hard error XT1007**, names the field, lists valid shapes |

## Reading

- **The prediction held on its core claim**: the wrong-class defect is invisible
  to every existing tool — it is exactly the check that typed declarations buy.
- **The prediction was wrong on one count, recorded as such**: the invalid unit
  is a HARD TeX error, not a soft warning. Hard-error count in existing tools: 2
  (< 4), so the stage's failure criterion is not met.
- **texlab is the strongest incumbent**: three defects get ERROR-severity editor
  squiggles. But nothing gates: no exit code, no build refusal — and the messages
  are generic ("Undefined reference") where xtex names the entity, its class,
  and the declaration site.
- **The build tool is the quietest**: with a broken reference, tectonic exits 0,
  prints progress notes to the terminal, ships a PDF containing "??", and the
  only trace of the problem is one line inside main.log.
