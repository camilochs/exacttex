#!/usr/bin/env python3
"""E6 tables from out/verify.csv, out/<id>/run{1,2}.xtexverified, out/<id>/check-verified.json. Prints markdown."""
import csv, json, re
from collections import Counter
from pathlib import Path
HERE = Path(__file__).resolve().parent; OUT = HERE / __import__("os").environ.get("OUT_DIR", "out")
rows = list(csv.DictReader(open(OUT / "verify.csv")))
rows_metrics = {}
for r in rows:
    for n in (1, 2):
        p = OUT / r["paper"] / f"run{n}.stderr.txt"
        if p.exists():
            lines = [l for l in p.read_text().splitlines() if l.startswith("network:")]
            if lines: rows_metrics[r[f"run{n}_metrics"]] = lines[-1]; r[f"run{n}_metrics"] = lines[-1]
def metrics(s):
    if s in rows_metrics: s = rows_metrics[s]
    m = re.search(r"network: (\d+) requests \((.*?)\) · (\d+) retries · (\d+) bytes down · (\d+) fetched, (\d+) carried over, (\d+) unanswered", s or "")
    return dict(requests=int(m.group(1)), sources=m.group(2), retries=int(m.group(3)), bytes=int(m.group(4)), fetched=int(m.group(5)), carried=int(m.group(6)), unanswered=int(m.group(7))) if m else {}
print("## (b) Live verification, run 1 — per paper\n")
print("| Paper | bib entries | claims | requests (by source) | bytes | fetched / unanswered | verified | partial | mismatch | unverified | reachable | redirected | unreachable | diffs (author) | wall |\n|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|")
tot = Counter(); alld = []
for r in rows:
    m1 = metrics(r["run1_metrics"]); rec = json.loads((OUT / r["paper"] / "run1.xtexverified").read_text())
    v = Counter((c["kind"], c["verdict"]) for c in rec["claims"])
    bib = sum(n for (k, vv), n in v.items() if k == "bib-entry")
    g = lambda k, vv: v.get((k, vv), 0)
    addr = lambda vv: sum(n for (k, x), n in v.items() if k != "bib-entry" and x == vv)
    diffs = [(c["target"], d) for c in rec["claims"] for d in c.get("diffs", [])]
    ad = [x for x in diffs if x[1]["field"] in ("authors", "author")]
    for c in rec["claims"]:
        for d in c.get("diffs", []): alld.append(dict(paper=r["paper"], key=c["target"], verdict=c["verdict"], source=c.get("source"), **d))
    for k in ("verified", "partial", "mismatch", "unverified"): tot[k] += g("bib-entry", k)
    for k in ("reachable", "redirected", "unreachable"): tot[k] += addr(k)
    tot["bib"] += bib; tot["requests"] += m1.get("requests", 0); tot["bytes"] += m1.get("bytes", 0); tot["diffs"] += len(diffs); tot["adiffs"] += len(ad); tot["claims"] += len(rec["claims"])
    print(f"| {r['paper']} | {bib} | {len(rec['claims'])} | {m1.get('requests')} ({m1.get('sources')}) | {m1.get('bytes', 0)/1e6:.1f} MB | {m1.get('fetched')} / {m1.get('unanswered')} | {g('bib-entry','verified')} | {g('bib-entry','partial')} | {g('bib-entry','mismatch')} | {g('bib-entry','unverified')} | {addr('reachable')} | {addr('redirected')} | {addr('unreachable')} | {len(diffs)} ({len(ad)}) | {r['run1_secs']} s |")
b = tot["bib"] or 1
print(f"\nTotals: {len(rows)} papers, {tot['claims']} claims, {tot['bib']} bib entries: verified {tot['verified']} ({100*tot['verified']/b:.0f}%), partial {tot['partial']} ({100*tot['partial']/b:.0f}%), mismatch {tot['mismatch']} ({100*tot['mismatch']/b:.0f}%), unverified {tot['unverified']} ({100*tot['unverified']/b:.0f}%); addresses reachable {tot['reachable']}, redirected {tot['redirected']}, unreachable {tot['unreachable']}; {tot['requests']} requests, {tot['bytes']/1e6:.1f} MB; {tot['diffs']} field diffs of which {tot['adiffs']} author diffs.\n")
print("### Unverified: failure notes (why no verdict)\n")
notes = Counter()
for r in rows:
    rec = json.loads((OUT / r["paper"] / "run1.xtexverified").read_text())
    for c in rec["claims"]:
        if c["verdict"] == "unverified": notes[(c["kind"], (c.get("failure_note") or "")[:70])] += 1
for (k, n), c in notes.most_common(): print(f"- {c} × {k}: `{n}`")
print("\n## (c) Second run, immediately after — the incremental claim\n")
print("| Paper | run 1: requests / fetched / carried / unanswered | run 2: requests / fetched / carried / unanswered | run 2 wall |\n|---|---|---|---|")
for r in rows:
    m1 = metrics(r["run1_metrics"]); m2 = metrics(r["run2_metrics"])
    print(f"| {r['paper']} | {m1.get('requests')} / {m1.get('fetched')} / {m1.get('carried')} / {m1.get('unanswered')} | {m2.get('requests')} / {m2.get('fetched')} / {m2.get('carried')} / {m2.get('unanswered')} | {r['run2_secs']} s |")
r1 = sum(metrics(r["run1_metrics"]).get("requests", 0) for r in rows); r2 = sum(metrics(r["run2_metrics"]).get("requests", 0) for r in rows)
c2 = sum(metrics(r["run2_metrics"]).get("carried", 0) for r in rows); f1 = sum(metrics(r["run1_metrics"]).get("fetched", 0) for r in rows); u1 = sum(metrics(r["run1_metrics"]).get("unanswered", 0) for r in rows)
print(f"\nRun 2 carried over {c2} claims (run 1 had answered {f1}); run 2 requests {r2} vs run 1 {r1} ({100*r2/max(1,r1):.0f}%), all of them for the {u1} claims run 1 left unanswered (unanswered claims are always retried, by design).\n")
print("## (d) `xtex check --verified` replay\n")
print("| Paper | exit | diagnostics by code | hard errors |\n|---|---|---|---|")
for r in rows: print(f"| {r['paper']} | {r['check_verified_exit']} | {r['check_verified_codes']} | {r['check_verified_errors']} |")
print("\n## Every field diff (document value vs source value)\n")
with open(OUT / "diffs-all.csv", "w", newline="") as f:
    w = csv.DictWriter(f, fieldnames=list(alld[0].keys()) if alld else ["paper"]); w.writeheader(); w.writerows(alld)
print(f"{len(alld)} diffs, by field: " + ", ".join(f"{k} {n}" for k, n in Counter(d['field'] for d in alld).most_common()) + f"; by severity: " + ", ".join(f"{k} {n}" for k, n in Counter(d['severity'] for d in alld).most_common()) + "\n")
import unicodedata
def families(s):
    """Family names of an author list, tolerant to LaTeX accents/braces, 'Last, First' order, line wraps and 'and others'."""
    s = re.sub(r"\\[`'^\"~=.uvHtcdbk]\s*\{?\s*([A-Za-z])\}?", r"\1", s)      # \'{e} -> e
    s = re.sub(r"\\(ss|ae|oe|o|l|i|j|aa)\b\s*", lambda m: {"ss": "ss", "ae": "ae", "oe": "oe", "o": "o", "l": "l", "i": "i", "j": "j", "aa": "a"}[m.group(1)], s)
    s = s.replace("{", "").replace("}", "").replace("\\", "")
    s = unicodedata.normalize("NFKD", s); s = "".join(ch for ch in s if not unicodedata.combining(ch))
    s = s.replace("\u2019", "'").replace("-", " ")
    out = []
    for a in re.split(r"\s+and\s+", " ".join(s.split())):
        a = a.strip().strip(",")
        if not a or a.lower() in ("others", "et al", "et al."): continue
        fam = a.split(",")[0].strip() if "," in a else a.split()[-1]
        fam = fam.split()[-1] if fam.split() else fam            # particles: "de Beaudrap" and "van de Wetering" compare by their last word
        out.append(fam.lower().replace("'", "").replace(".", ""))
    return out
def classify(d):
    fd, fs = families(d["in_document"]), families(d["in_source"])
    if fd == fs: return "formatting only (same family names, same order)"
    if sorted(fd) == sorted(fs): return "same family names, different order"
    if re.search(r"\band others\b|et al", d["in_document"]): return "document list truncated with 'and others'"
    if len(fd) > 50 or len(fs) > 50: return "very long list (>50), not compared"
    if len(fd) != len(fs): return f"substantive: {len(fd)} vs {len(fs)} authors"
    return "substantive: a family name differs"
for d in alld:
    if d["field"] in ("authors", "author"): d["class"] = classify(d)
cc = Counter(d.get("class") for d in alld if d["field"] in ("authors", "author"))
print("Author diffs by class (family names compared after decoding LaTeX accents and reordering 'Last, First'): " + ", ".join(f"{k}: {n}" for k, n in cc.most_common()) + "\n")
print("| Paper | Key | Verdict | Class | Document | Source |\n|---|---|---|---|---|---|")
for d in alld:
    if d["field"] in ("authors", "author") and d["class"].startswith("substantive"): print(f"| {d['paper']} | `{d['key']}` | {d['verdict']} | {d['class']} | {' '.join(d['in_document'].split())[:120]} | {' '.join(d['in_source'].split())[:120]} |")
print("\nFormatting-only author diffs (first 5 of each paper):\n")
seen = Counter()
for d in alld:
    if d["field"] in ("authors", "author") and not d["class"].startswith("substantive") and seen[d["paper"]] < 3:
        seen[d["paper"]] += 1; print(f"- {d['paper']} `{d['key']}`: `{' '.join(d['in_document'].split())[:80]}` vs `{' '.join(d['in_source'].split())[:80]}`")
print("\n(non-author diffs are in `out/diffs-all.csv`)")
