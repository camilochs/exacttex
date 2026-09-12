#!/usr/bin/env python3
"""E6 (d) replay with a given compiler on the EXISTING records (work/<id>/.xtexverified from the live run).
Usage: XTEX=... OUT_DIR=out-<commit> python3 replay.py"""
import csv, json, os, subprocess, time
from collections import Counter
from pathlib import Path
HERE = Path(__file__).resolve().parent; C = HERE.parent
XTEX = os.environ["XTEX"]; OUT = HERE / os.environ.get("OUT_DIR", "out"); OUT.mkdir(exist_ok=True)
man = {r["id"]: r for r in csv.DictReader(open(C / "manifest.csv"))}
rows = []
for r in csv.DictReader(open(HERE / "out" / "verify.csv")):
    pid = r["paper"]; wd = HERE / "work" / pid; root = man[pid]["main_file"][:-4] + ".xtex"
    t = time.time(); p = subprocess.run([XTEX, "check", "--verified", "--json", root], cwd=wd, capture_output=True, text=True, timeout=600); secs = time.time() - t
    (OUT / pid).mkdir(exist_ok=True); (OUT / pid / "check-verified.json").write_text(p.stdout)
    try:
        j = json.loads(p.stdout.strip().splitlines()[-1]); sev = Counter((d["code"], d["severity"]) for d in j["diagnostics"])
        rows.append(dict(paper=pid, exit=p.returncode, secs=round(secs, 2), **{f"{c}_{s}": n for (c, s), n in sev.items()}))
    except Exception: rows.append(dict(paper=pid, exit=p.returncode, error=(p.stdout + p.stderr)[-200:]))
    print(rows[-1], flush=True)
keys = []
for r in rows:
    for k in r:
        if k not in keys: keys.append(k)
with open(OUT / "replay.csv", "w", newline="") as f:
    w = csv.DictWriter(f, fieldnames=keys); w.writeheader(); w.writerows(rows)
