#!/bin/bash
# Stage 0b: for each Class-A defect, what does each existing tool say —
# hard error (build fails / error diagnostic), warning, or nothing?
# The prediction was registered in the plan BEFORE this ran.
#
# Usage: ./run.sh [main|clean]   (default: main — the defective file;
# `clean` runs the defect-free twin of every case as a control)
#
# tectonic, chktex and chklref run on <stem>.tex; xtex on <stem>.xtex. texlab
# is driven separately by texlab-probe.mjs, which speaks LSP to it.
set -uo pipefail
stem="${1:-main}"
cd "$(dirname "$0")/cases"
xtex="$(cd ../../../.. && pwd)/target/debug/xtex"
for case in broken-ref missing-cite duplicate-label wrong-class prose-word missing-figure invalid-unit; do
  echo "=== $case ($stem)"
  cd "$case"
  # tectonic: exit code + errors/warnings in output
  out=$(tectonic --chatter minimal "$stem.tex" 2>&1); code=$?
  echo "--- tectonic exit=$code"
  echo "$out" | grep -iE "error|warning|undefined|not found|missing" | head -4
  # chktex
  out=$(chktex -q "$stem.tex" 2>&1); echo "--- chktex exit=$?"
  echo "$out" | head -4
  # chklref drives pdflatex itself and prints its report after the engine's
  # chatter. The report is what matters: every section header and every
  # finding line, with the engine's own transcript filtered out — except the
  # engine's warnings and errors, printed once each and marked as relayed,
  # because the run.sh that preceded this one cut the whole report off with
  # `head -4` and recorded chklref as silent.
  out=$(chklref "$stem.tex" 2>&1); echo "--- chklref exit=$?"
  echo "$out" | sed -n '/^\*\*\*/,$p' | grep -vE "^\s*$" | grep -vE "^\*+$"
  echo "$out" | grep -E "^(LaTeX Warning|!)" | sort -u
  # xtex, on the annotated twin
  out=$("$xtex" check "$stem.xtex" 2>&1); echo "--- xtex exit=$?"
  echo "$out" | grep -vE "^\s+(span|blame):" | head -8
  rm -f "$stem.aux" "$stem.log" "$stem.pdf" "$stem.bbl" "$stem.blg" "$stem.out" 2>/dev/null
  cd ..
done
