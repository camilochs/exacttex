#!/bin/bash
# Stage 0b: for each Class-A defect, what does each existing tool say —
# hard error (build fails / error diagnostic), warning, or nothing?
# The prediction was registered in the plan BEFORE this ran.
set -uo pipefail
cd "$(dirname "$0")/cases"
for case in broken-ref missing-cite duplicate-label wrong-class missing-figure invalid-unit; do
  echo "=== $case"
  cd "$case"
  # tectonic: exit code + errors/warnings in output
  out=$(tectonic --chatter minimal main.tex 2>&1); code=$?
  echo "--- tectonic exit=$code"
  echo "$out" | grep -iE "error|warning|undefined|not found|missing" | head -4
  # chktex
  out=$(chktex -q main.tex 2>&1); echo "--- chktex exit=$?"
  echo "$out" | head -4
  # chklref (needs a compiled aux; run after tectonic which kept none — run with its own latex? chklref drives pdflatex internally)
  out=$(chklref main.tex 2>&1); echo "--- chklref exit=$?"
  echo "$out" | grep -viE "^\s*$" | head -4
  rm -f main.aux main.log main.pdf main.bbl main.blg main.out 2>/dev/null
  cd ..
done
