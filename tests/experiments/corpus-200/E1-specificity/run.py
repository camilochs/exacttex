#!/usr/bin/env python3
"""E1: rename every .tex to .xtex unmodified, `xtex check` + `xtex build`, compare bytes.
Writes out/files.csv (one row per file), out/diagnostics.jsonl (every diagnostic, if any),
out/diffs/ (unified diffs for any non-identical emission). Deterministic; re-runnable."""
import csv, json, os, shutil, subprocess, sys, time
from pathlib import Path

HERE = Path(__file__).resolve().parent
CORPUS = HERE.parent
XTEX = Path(os.environ.get("XTEX", str(Path(__file__).resolve().parents[1] / "bin" / "xtex-d0e0caa")))
WORK = HERE / os.environ.get("WORK_DIR", "work"); OUT = HERE / os.environ.get("OUT_DIR", "out")
TIMEOUT = int(os.environ.get("TIMEOUT", "120"))

def run(cmd, cwd):
    t = time.time()
    try:
        p = subprocess.run(cmd, cwd=cwd, capture_output=True, text=True, timeout=TIMEOUT)
        return p.returncode, p.stdout, p.stderr, time.time() - t
    except subprocess.TimeoutExpired:
        return "timeout", "", "", TIMEOUT

def main():
    papers = [r for r in csv.DictReader(open(CORPUS / "manifest.csv"))]
    if WORK.exists(): shutil.rmtree(WORK)
    OUT.mkdir(exist_ok=True); (OUT / "diffs").mkdir(exist_ok=True)
    for f in (OUT / "diffs").glob("*"): f.unlink()
    rows = []; diags = open(OUT / "diagnostics.jsonl", "w")
    for r in papers:
        pid = r["id"]; src = CORPUS / "papers" / pid; wd = WORK / pid
        shutil.copytree(src, wd, symlinks=False)
        texs = sorted(p.relative_to(wd) for p in wd.rglob("*.tex") if p.is_file())
        for rel in texs:  # rename ALL first so \input targets resolve to the renamed copies
            (wd / rel).rename((wd / rel).with_suffix(".xtex"))
        for rel in texs:
            xrel = rel.with_suffix(".xtex")
            orig = (src / rel).read_bytes()
            code, out, err, secs = run([str(XTEX), "check", "--json", str(xrel)], wd)
            n_err = n_adv = 0; coverage = ""; bib = ""; parsed = False
            if out.strip():
                try:
                    j = json.loads(out.splitlines()[-1]); parsed = True
                    coverage = j.get("coverage", ""); bib = j.get("bibliography", {}).get("state", "")
                    for d in j.get("diagnostics", []):
                        diags.write(json.dumps(dict(paper=pid, file=str(rel), **d)) + "\n")
                        if d.get("severity") == "error": n_err += 1
                        else: n_adv += 1
                except json.JSONDecodeError: pass
            bcode, bout, berr, bsecs = run([str(XTEX), "build", str(xrel)], wd)
            emitted = wd / "build" / rel
            identical = ""; bytes_diff = ""
            if emitted.exists():
                em = emitted.read_bytes(); identical = em == orig
                if not identical:
                    import difflib
                    d = difflib.unified_diff(orig.decode("utf-8", "replace").splitlines(True), em.decode("utf-8", "replace").splitlines(True), "original", "emitted")
                    (OUT / "diffs" / f"{pid}__{str(rel).replace('/', '_')}.diff").write_text("".join(d))
                    bytes_diff = sum(1 for a, b in zip(orig, em) if a != b) + abs(len(orig) - len(em))
            rows.append(dict(paper=pid, file=str(rel), bytes=len(orig), check_exit=code, check_json=parsed, errors=n_err, advisories=n_adv,
                             coverage=coverage, bibliography=bib, check_stderr=err.strip()[:200], check_secs=f"{secs:.2f}",
                             build_exit=bcode, emitted=emitted.exists(), identical=identical, bytes_differing=bytes_diff, build_stderr=berr.strip()[:200]))
            print(pid, rel, "check", code, "err", n_err, "adv", n_adv, "cov", coverage, "build", bcode, "identical", identical, flush=True)
    diags.close()
    with open(OUT / "files.csv", "w", newline="") as f:
        w = csv.DictWriter(f, fieldnames=list(rows[0].keys())); w.writeheader(); w.writerows(rows)
    print("files:", len(rows))

if __name__ == "__main__":
    main()
