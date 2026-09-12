#!/bin/sh
# One paper per call, so a run that is cut short loses one paper and not the
# campaign: run.py merges each paper's rows into out/results.csv, and this
# driver asks only for the papers that are not in there yet.
set -eu
here="$(cd "$(dirname "$0")" && pwd)"
cd "$here"
export XTEX="${XTEX:-$here/../bin/xtex-9731d47}"
while :; do
    next="$(python3 - <<'PY'
import csv, pathlib
want = [r["id"] for r in csv.DictReader(open("subsample.csv"))]
have = set()
p = pathlib.Path("out/results.csv")
if p.exists():
    have = {r["paper"] for r in csv.DictReader(p.open())}
missing = [i for i in want if i not in have]
print(missing[0] if missing else "")
PY
)"
    [ -n "$next" ] || { echo "drive: every paper of the subsample is in results.csv"; break; }
    echo "drive: $next"
    python3 -u run.py "$next" || echo "drive: $next failed, moving on"
done
