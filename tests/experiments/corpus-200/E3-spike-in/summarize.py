#!/usr/bin/env python3
"""E3 recall tables from out/results.csv: per tool × class, counts, Wilson 95% CI, paired exact McNemar (one-sided)
for xtex vs texlab and xtex vs tectonic (gating). Prints markdown."""
import csv, math
from collections import defaultdict
from pathlib import Path
HERE = Path(__file__).resolve().parent
import os
rows = list(csv.DictReader(open(HERE / os.environ.get("OUT_DIR", "out") / "results.csv")))
CLASSES = ["A1-broken-ref", "A2-missing-cite", "A3-duplicate-id", "A4-wrong-class", "A5-missing-figure", "A6-invalid-unit"]
T = lambda v: v == "True"

def wilson(k, n, z=1.959964):
    if n == 0: return (float("nan"), float("nan"))
    p = k / n; d = 1 + z * z / n; c = p + z * z / (2 * n); h = z * math.sqrt(p * (1 - p) / n + z * z / (4 * n * n))
    return ((c - h) / d, (c + h) / d)

def mcnemar_one_sided(b, c):
    """b = xtex hit & other miss, c = xtex miss & other hit. One-sided exact binomial P(X >= b | n=b+c, 1/2)."""
    n = b + c
    if n == 0: return float("nan")
    return sum(math.comb(n, i) for i in range(b, n + 1)) / 2 ** n

def fmt(k, n):
    if n == 0: return "—"
    lo, hi = wilson(k, n); return f"{k}/{n} = {100*k/n:.0f}% [{100*lo:.0f}, {100*hi:.0f}]"

papers = sorted({r["paper"] for r in rows})
skipped = [r for r in rows if r["skip"]]
print(f"Papers: {len(papers)}. Planted twins: {sum(1 for r in rows if not r['skip'])}. Skipped (class, reason): " + "; ".join(f"{r['paper']} {r['cls']}: {r['skip']}" for r in skipped) + "\n")
print("## Recall per tool and defect class — gating level (95% Wilson interval)\n")
print("| Class | n planted | xtex (error, exit 1) | texlab (ERROR) | tectonic (non-zero exit) | tectonic (any: exit or log warning) | texlab (any severity) |\n|---|---|---|---|---|---|---|")
for c in CLASSES:
    R = [r for r in rows if r["cls"] == c and not r["skip"]]
    n = len(R)
    tec = [r for r in R if r["tectonic_exit"] not in ("n/a", "")]
    print(f"| {c} | {n} | {fmt(sum(T(r['xtex_detected']) for r in R), n)} | {fmt(sum(T(r['texlab_gating']) for r in R), n)} | {fmt(sum(T(r['tectonic_gating']) for r in tec), len(tec))} | {fmt(sum(T(r['tectonic_any']) for r in tec), len(tec))} | {fmt(sum(T(r['texlab_any']) for r in R), n)} |")
print("\ntectonic columns use only papers whose clean source builds under tectonic (denominator shown).\n")
print("## Paired comparison, gating level (H0: xtex recall ≤ other tool's recall; exact one-sided McNemar on discordant pairs)\n")
print("| Class | xtex>texlab | texlab>xtex | p (xtex vs texlab) | xtex>tectonic | tectonic>xtex | p (xtex vs tectonic) |\n|---|---|---|---|---|---|---|")
for c in CLASSES:
    R = [r for r in rows if r["cls"] == c and not r["skip"]]
    b1 = sum(1 for r in R if T(r["xtex_detected"]) and not T(r["texlab_gating"])); c1 = sum(1 for r in R if not T(r["xtex_detected"]) and T(r["texlab_gating"]))
    tec = [r for r in R if r["tectonic_exit"] not in ("n/a", "")]
    b2 = sum(1 for r in tec if T(r["xtex_detected"]) and not T(r["tectonic_gating"])); c2 = sum(1 for r in tec if not T(r["xtex_detected"]) and T(r["tectonic_gating"]))
    p1 = mcnemar_one_sided(b1, c1); p2 = mcnemar_one_sided(b2, c2)
    print(f"| {c} | {b1} | {c1} | {'—' if math.isnan(p1) else f'{p1:.2g}'} | {b2} | {c2} | {'—' if math.isnan(p2) else f'{p2:.2g}'} |")
print("\n## Conditional recall for A2 (missing key)\n")
R = [r for r in rows if r["cls"] == "A2-missing-cite" and not r["skip"]]
comp = [r for r in R if T(r["bib_complete"])]; unav = [r for r in R if not T(r["bib_complete"])]
print(f"- xtex, bibliography `complete`: {fmt(sum(T(r['xtex_detected']) for r in comp), len(comp))}; bibliography `unavailable`: {fmt(sum(T(r['xtex_detected']) for r in unav), len(unav))} (by design: no key may be called missing from an unread bibliography)")
print(f"- texlab, same split: complete {fmt(sum(T(r['texlab_gating']) for r in comp), len(comp))}; unavailable {fmt(sum(T(r['texlab_gating']) for r in unav), len(unav))}")
tec = [r for r in R if r["tectonic_exit"] not in ("n/a", "")]
print(f"- tectonic any-signal, same split: complete {fmt(sum(T(r['tectonic_any']) for r in [x for x in tec if T(x['bib_complete'])]), len([x for x in tec if T(x['bib_complete'])]))}; unavailable {fmt(sum(T(r['tectonic_any']) for r in [x for x in tec if not T(x['bib_complete'])]), len([x for x in tec if not T(x['bib_complete'])]))}")
print("\n## Misses by xtex (each is a candidate compiler bug)\n")
miss = [r for r in rows if not r["skip"] and not T(r["xtex_detected"])]
for r in miss: print(f"- {r['paper']} {r['cls']} planted `{r['planted']}` at {r['sites']} — exit {r['xtex_exit']}, new diagnostics {r['xtex_new']}, bib complete {r['bib_complete']}; note: {r['note']}")
if not miss: print("(none)")
print("\n## tectonic baseline\n")
base = defaultdict(int)
for p in papers:
    rr = [r for r in rows if r["paper"] == p and not r["skip"]]
    if not rr: base["no twin"] += 1
    elif all(r["tectonic_exit"] in ("n/a", "") for r in rr): base["clean source does not build under tectonic"] += 1
    else: base["builds"] += 1
for k, v in base.items(): print(f"- {k}: {v} papers")
print("\n## xtex codes observed per class\n")
for c in CLASSES:
    codes = defaultdict(int)
    for r in rows:
        if r["cls"] == c and not r["skip"] and r["xtex_codes"]: codes[r["xtex_codes"]] += 1
    print(f"- {c}: " + ", ".join(f"{k} ×{v}" for k, v in codes.items()))
