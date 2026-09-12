#!/usr/bin/env python3
"""E6 (a): `xtex claims` on every annotated twin (E2 work dir). Writes out/<id>.claims.json and out/inventory.csv; prints the distribution."""
import json, csv, subprocess, statistics as st, os
from pathlib import Path
from collections import Counter
HERE = Path(__file__).resolve().parent; C = HERE.parent; E2 = C / "E2-annotation-cost"
XTEX = os.environ.get("XTEX", str(C / "bin" / "xtex-d0e0caa"))
OUT = HERE / os.environ.get("OUT_DIR", "out"); OUT.mkdir(exist_ok=True)
from concurrent.futures import ThreadPoolExecutor
def one(r):
    pid = r["id"]; wd = E2 / "work" / pid; root = r["main_file"][:-4] + ".xtex"
    import time; t0 = time.time()
    try: p = subprocess.run([XTEX, "claims", root], cwd=wd, capture_output=True, text=True, timeout=int(os.environ.get("CLAIMS_TIMEOUT", "300")))
    except subprocess.TimeoutExpired: print(pid, "TIMEOUT", flush=True); return dict(paper=pid, error=f"timeout after {os.environ.get('CLAIMS_TIMEOUT', '300')} s")
    try: j = json.loads(p.stdout)
    except Exception: return dict(paper=pid, error=p.stderr[-200:])
    print(pid, f"{time.time()-t0:.1f}s", len(j["claims"]), "claims", flush=True)
    kinds = Counter(c["kind"] for c in j["claims"])
    bib = [c for c in j["claims"] if c["kind"] == "bib-entry"]
    (OUT / f"{pid}.claims.json").write_text(json.dumps(j))
    return dict(paper=pid, seconds=round(time.time() - t0, 1), bib_entries=len(bib), bib_with_doi=sum(1 for c in bib if c.get("fields", {}).get("doi")),
                     bib_with_url=sum(1 for c in bib if c.get("fields", {}).get("url")), bib_mentioning_arxiv=sum(1 for c in bib if "arxiv" in json.dumps(c.get("fields", {})).lower()),
                     urls=kinds.get("url", 0), dois=kinds.get("doi", 0), repositories=kinds.get("repository", 0))
with ThreadPoolExecutor(max_workers=int(os.environ.get("WORKERS", "3"))) as ex:
    rows = list(ex.map(one, list(csv.DictReader(open(C / "manifest.csv")))))
with open(OUT / "inventory.csv", "w", newline="") as f:
    keys = []
    for r in rows:
        for k in r:
            if k not in keys: keys.append(k)
    w = csv.DictWriter(f, fieldnames=keys); w.writeheader(); w.writerows(rows)
ok = [r for r in rows if "bib_entries" in r]
print("| Claim kind | Sum | Median per paper | Min | Max | Papers with ≥1 |\n|---|---|---|---|---|---|")
for k, lab in [("bib_entries", "bib entries"), ("bib_with_doi", "bib entries with a `doi` field"), ("bib_with_url", "bib entries with a `url` field"), ("bib_mentioning_arxiv", "bib entries mentioning arXiv in any field"), ("urls", "`url` claims (typed blocks)"), ("dois", "`doi` claims"), ("repositories", "`repository` claims")]:
    v = [r[k] for r in ok]; print(f"| {lab} | {sum(v)} | {st.median(v):g} | {min(v)} | {max(v)} | {sum(1 for x in v if x > 0)} |")
print("\nerrors:", [r for r in rows if "error" in r])
print("\nTop papers by DOI-bearing entries:", [(r["paper"], r["bib_with_doi"], r["bib_entries"]) for r in sorted(ok, key=lambda r: -r["bib_with_doi"])[:12]])
