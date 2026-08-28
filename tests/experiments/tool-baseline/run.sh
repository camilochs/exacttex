#!/usr/bin/env bash
# Reproduce the tool baseline. Writes raw/<fixture>.<tool>.{txt,json}.
set -u
cd "$(dirname "$0")"
mkdir -p raw
for d in fixtures/*/; do
  n=$(basename "$d")
  ( cd "$d"
    tectonic -X compile --print main.tex > "../../raw/$n.tectonic.txt" 2>&1
    echo "TECTONIC_EXIT=$?" >> "../../raw/$n.tectonic.txt"
    chktex -q main.tex > "../../raw/$n.chktex.txt" 2>&1
    echo "CHKTEX_EXIT=$?" >> "../../raw/$n.chktex.txt"
    chklref main.tex > "../../raw/$n.chklref.txt" 2>&1
    echo "CHKLREF_EXIT=$?" >> "../../raw/$n.chklref.txt" )
  python3 texlab_probe.py "$d/main.tex" 4 > "raw/$n.texlab.json" 2>/dev/null
done
echo "hard failures (non-zero tectonic exit):"
grep -l "TECTONIC_EXIT=[^0]" raw/*.tectonic.txt | sed 's|raw/|  |;s|\.tectonic\.txt||'
