#!/usr/bin/env python3
"""E6 (b)(c)(d): live external verification on the 10 papers with the most DOI-bearing bib entries.
Sequential, one paper at a time. Run 1 writes .xtexverified beside each twin root; run 2 repeats immediately
(incremental claim); then `xtex check --verified`. Everything is recorded under out/<id>/.
Usage: verify.py [--top=10] [--budget-min=30] [paper-id ...]"""
import csv, json, os, re, shutil, subprocess, sys, time
from pathlib import Path
from collections import Counter
HERE = Path(__file__).resolve().parent; C = HERE.parent; E2 = C / "E2-annotation-cost"
XTEX = os.environ.get("XTEX", str(C / "bin" / "xtex-d0e0caa"))
XVERIFY = os.environ.get("XVERIFY", str(C / "bin" / "xtex-verify-d0e0caa"))
OUT = HERE / "out"; WORK = HERE / "work"; OUT.mkdir(exist_ok=True); WORK.mkdir(exist_ok=True)
MAILTO = "corrigendum.lab@gmail.com"

def sh(cmd, cwd, timeout):
    t = time.time()
    try:
        p = subprocess.run(cmd, cwd=cwd, capture_output=True, text=True, timeout=timeout, errors="replace")
        return p.returncode, p.stdout, p.stderr, time.time() - t
    except subprocess.TimeoutExpired as e:
        return "timeout", (e.stdout or ""), (e.stderr or ""), time.time() - t

def metrics_line(stderr):
    lines = [l for l in stderr.splitlines() if l.strip()]
    return lines[-1] if lines else ""

def summarize_record(path):
    """Verdict distribution and every per-field diff from a .xtexverified record (tolerant to the exact shape)."""
    try: rec = json.loads(Path(path).read_text())
    except Exception as e: return dict(error=f"unreadable record: {e!r}"), []
    verdicts = Counter(); diffs = []; unverified_notes = Counter(); n = 0
    items = rec.get("claims") or rec.get("verdicts") or rec.get("entries") or (rec if isinstance(rec, list) else [])
    if isinstance(items, dict): items = [dict(target=k, **(v if isinstance(v, dict) else {"verdict": v})) for k, v in items.items()]
    for it in items:
        n += 1
        v = it.get("verdict") or it.get("status") or "?"
        kind = it.get("kind", "?"); verdicts[(kind, v)] += 1
        if v == "unverified": unverified_notes[(it.get("failure_note") or it.get("failure") or it.get("note") or "")[:90]] += 1
        for d in it.get("diffs") or it.get("fields_differ") or []:
            if isinstance(d, dict):
                diffs.append(dict(target=it.get("target"), kind=kind, verdict=v, field=d.get("field"), severity=d.get("severity"), document=str(d.get("in_document") or d.get("document") or "")[:200], source=str(d.get("in_source") or d.get("source") or "")[:200]))
    return dict(claims=n, verdicts={f"{k}:{v}": c for (k, v), c in verdicts.items()}, unverified_notes=dict(unverified_notes), raw_keys=list(rec.keys())[:10] if isinstance(rec, dict) else "list"), diffs

def main():
    args = sys.argv[1:]
    top = int(next((a.split("=")[1] for a in args if a.startswith("--top=")), 10))
    budget = float(next((a.split("=")[1] for a in args if a.startswith("--budget-min=")), 30)) * 60
    max_entries = int(next((a.split("=")[1] for a in args if a.startswith("--max-entries=")), 10**9))
    ids = [a for a in args if not a.startswith("--")]
    man = {r["id"]: r for r in csv.DictReader(open(C / "manifest.csv"))}
    inv = [r for r in csv.DictReader(open(HERE / "out" / "inventory.csv")) if r.get("bib_with_doi", "") != ""]
    if not ids:
        inv = [r for r in inv if int(r["bib_entries"]) <= max_entries]
        inv.sort(key=lambda r: (-int(r["bib_with_doi"]), r["paper"])); ids = [r["paper"] for r in inv[:top]]
    print("selected:", ids, flush=True)
    rows = []; t_start = time.time(); stopped = ""
    for pid in ids:
        if time.time() - t_start > budget: stopped = f"budget of {budget/60:.0f} min exhausted before {pid}"; print(stopped); break
        wd = WORK / pid
        if wd.exists(): shutil.rmtree(wd)
        shutil.copytree(E2 / "work" / pid, wd, ignore=shutil.ignore_patterns("build"))
        root = man[pid]["main_file"][:-4] + ".xtex"; po = OUT / pid; po.mkdir(exist_ok=True)
        row = dict(paper=pid)
        for run in (1, 2):
            code, out, err, secs = sh([XVERIFY, f"--mailto={MAILTO}", root], wd, 900)
            (po / f"run{run}.stdout.txt").write_text(out); (po / f"run{run}.stderr.txt").write_text(err)
            row[f"run{run}_exit"] = code; row[f"run{run}_secs"] = round(secs, 1); row[f"run{run}_metrics"] = metrics_line(err)
            recs = [p for p in [wd / Path(root).parent / ".xtexverified", wd / ".xtexverified"] if p.exists()] + list(wd.rglob("*.xtexverified"))
            if recs: shutil.copy(recs[0], po / f"run{run}.xtexverified")
            print(pid, f"run{run}", code, f"{secs:.0f}s", metrics_line(err)[:160], flush=True)
            if re.search(r"(?<![\d./-])429(?![\d-])", err) or "rate limit" in err.lower() or "too many requests" in err.lower(): stopped = f"rate limit reported on {pid} run {run}"; print(stopped); break
        rec = po / "run1.xtexverified"
        if rec.exists():
            summ, diffs = summarize_record(rec)
            row.update(claims=summ.get("claims"), verdicts=json.dumps(summ.get("verdicts")), unverified_notes=json.dumps(summ.get("unverified_notes")), record_keys=json.dumps(summ.get("raw_keys")))
            with open(po / "diffs.csv", "w", newline="") as f:
                if diffs:
                    w = csv.DictWriter(f, fieldnames=list(diffs[0].keys())); w.writeheader(); w.writerows(diffs)
            row["diffs"] = len(diffs); row["author_diffs"] = sum(1 for d in diffs if (d.get("field") or "").lower().startswith("author"))
        # (d) replay
        code, out, err, secs = sh([XTEX, "check", "--verified", "--json", root], wd, 600)
        (po / "check-verified.json").write_text(out); (po / "check-verified.stderr.txt").write_text(err)
        try:
            j = json.loads(out.strip().splitlines()[-1]); codes = Counter(d["code"] for d in j.get("diagnostics", []))
            row["check_verified_exit"] = code; row["check_verified_codes"] = json.dumps(codes); row["check_verified_errors"] = sum(1 for d in j.get("diagnostics", []) if d.get("severity") == "error")
        except Exception: row["check_verified_exit"] = code; row["check_verified_codes"] = "unparsed: " + (out + err)[-200:]
        rows.append(row)
        if stopped: break
    if (OUT / "verify.csv").exists():   # merge: rows for papers not in this run are kept
        done = {r["paper"] for r in rows}
        rows = [r for r in csv.DictReader(open(OUT / "verify.csv")) if r["paper"] not in done] + rows
    keys = []
    for r in rows:
        for k in r:
            if k not in keys: keys.append(k)
    with open(OUT / "verify.csv", "w", newline="") as f:
        w = csv.DictWriter(f, fieldnames=keys); w.writeheader(); w.writerows(rows)
    (OUT / "verify-run.txt").write_text(f"selected={ids}\nstopped={stopped or 'no'}\nwall={time.time()-t_start:.0f}s\n")
    print("done; stopped:", stopped or "no", f"wall {time.time()-t_start:.0f}s")

if __name__ == "__main__":
    main()
