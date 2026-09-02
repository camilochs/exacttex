#!/usr/bin/env python3
"""Compares `xtex adopt` against the corpus experiment's converted twins (E2).

The E2 script converted 50 arXiv papers with regular expressions, then
reverted, one token at a time, whatever `xtex check` reported as a hard error
and whatever the emitter wrote back literally. Its twins are the reference
this command must reproduce, with one class of difference expected: a token
the script reverted because `xtex check` reported it (a duplicate label, a
reference to nothing, a key absent from the bibliography) is a token adopt
keeps, because adopt converts and does not check. Every such difference is
matched here against the experiment's own record of what it reverted and why;
anything else is printed as unexplained and fails the run.

Usage:
  adopt-twins.py --xtex target/release/xtex --papers CORPUS/papers \\
      --manifest CORPUS/manifest.csv --twins E2/work-<commit> \\
      --results E2/out-<commit> --scratch /tmp/adopt-twins

Nothing under --papers or --twins is written; each paper is copied to
--scratch and adopted there.
"""
import argparse, csv, difflib, json, re, shutil, subprocess, sys
from pathlib import Path

RX_TOKEN = re.compile(r"@(cite|citep|citet|textcite|parencite|id|ref)\(([^)\n]*)\)")

# Two defects of the E2 script's regular-expression scanner, located with the
# script's own `live_spans` over the original files (2026-09-02). In each the
# script read a `$` as opening math where TeX does not, and left every
# construct up to the next `$` unconverted; the compiler's scanner reads
# both correctly, so adopt converts there and the twin does not. A file
# listed here may differ from its twin only in that direction.
SCRIPT_DEFECTS = {
    ("2501.00169", "DeepLL.xtex"): "`\\AxiomC{$$}` (bussproofs) read as display math opening at `$$`, dead to the next `$$` (lines 531-633)",
    ("2608.27798", "main.xtex"): "`$` inside `\\lstinline|$loc|` read as inline math; the script handles `\\verb` only",
}


def unconverted(token):
    kind, arg = RX_TOKEN.fullmatch(token).groups()
    if kind == "id":
        return "\\label{" + arg + "}"
    if kind == "ref":
        return "\\ref{" + arg + "}"
    return "\\" + kind + "{" + arg + "}"


def explained_by_revert(actual_line, twin_line, reverts):
    """Whether `twin_line` is `actual_line` with recorded reverts applied."""
    line = actual_line
    used = []
    for m in RX_TOKEN.finditer(actual_line):
        token = m.group(0)
        kind, arg = m.groups()
        keys = arg.split(",") if kind not in ("id", "ref") else [arg]
        for rev in reverts:
            if rev["name"] in keys and not rev.get("used"):
                candidate = line.replace(token, unconverted(token), 1)
                if candidate != line:
                    line = candidate
                    used.append(rev)
                break
    return line == twin_line, used


def explained_by_defect(actual_line, twin_line):
    """Whether the twin is the adopt line with every construct written back."""
    return RX_TOKEN.sub(lambda m: unconverted(m.group(0)), actual_line) == twin_line


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--xtex", required=True)
    ap.add_argument("--papers", required=True, type=Path)
    ap.add_argument("--manifest", required=True, type=Path)
    ap.add_argument("--twins", required=True, type=Path)
    ap.add_argument("--results", required=True, type=Path)
    ap.add_argument("--scratch", required=True, type=Path)
    ap.add_argument("ids", nargs="*")
    args = ap.parse_args()

    papers = {r["id"]: r for r in csv.DictReader(open(args.manifest))}
    ids = args.ids or list(papers)
    totals = dict(papers=0, files_identical=0, files_explained=0, files_unexplained=0,
                  reverts_explained=0, files_crlf_normalized_by_script=0, files_script_defect=0,
                  extra_by_adopt=0, extra_by_script=0, adopt_failed=0)
    unexplained = []
    for pid in ids:
        main_rel = papers[pid]["main_file"]
        src = args.papers / pid
        wd = args.scratch / pid
        if wd.exists():
            shutil.rmtree(wd)
        shutil.copytree(src, wd, symlinks=False)
        run = subprocess.run([args.xtex, "adopt", "--json", main_rel], cwd=wd, capture_output=True, text=True)
        try:
            report = json.loads(run.stdout.strip().splitlines()[-1])
        except Exception:
            print(f"{pid}: adopt produced no report (exit {run.returncode}): {run.stderr[-300:]}")
            totals["adopt_failed"] += 1
            continue
        totals["papers"] += 1
        result = json.load(open(args.results / f"{pid}.json"))
        reverts = [d for d in result.get("defects", []) if d.get("reverted")]
        twin_files = {p.relative_to(args.twins / pid) for p in (args.twins / pid).rglob("*.xtex")}
        adopt_files = {Path(f["output"]) for f in report["files"] if f["converted"]}
        for rel in sorted(adopt_files - twin_files):
            print(f"{pid}: adopt converted {rel}, the script did not")
            totals["extra_by_adopt"] += 1
            unexplained.append((pid, str(rel), "extra file by adopt"))
        for rel in sorted(twin_files - adopt_files):
            reason = next((f.get("failure") for f in report["files"] if Path(f["output"]) == rel), "not reached")
            print(f"{pid}: the script converted {rel}, adopt did not: {reason}")
            totals["extra_by_script"] += 1
            unexplained.append((pid, str(rel), f"missing file: {reason}"))
        for rel in sorted(twin_files & adopt_files):
            twin = (args.twins / pid / rel).read_bytes()
            actual = (wd / rel).read_bytes()
            if twin == actual:
                totals["files_identical"] += 1
                continue
            if b"\r\n" in actual and twin == actual.replace(b"\r\n", b"\n"):
                # The script read and wrote text with Python's universal
                # newlines, so a CRLF file came back with LF endings. Adopt
                # keeps the bytes. Its own identity check compared normalized
                # text on both sides, so the experiment did not notice.
                totals["files_crlf_normalized_by_script"] += 1
                lost = actual.count(b"\r")
                print(f"{pid}/{rel}: identical once the script's CRLF -> LF normalization is undone ({lost} CR bytes lost by the script)")
                continue
            twin_lines = twin.split(b"\n")
            actual_lines = actual.split(b"\n")
            file_reverts = [dict(r) for r in reverts if r["file"] == str(rel)]
            problems = []
            differing = []
            if len(twin_lines) != len(actual_lines):
                problems.append(f"line count {len(actual_lines)} vs twin {len(twin_lines)}")
            else:
                for n, (a, t) in enumerate(zip(actual_lines, twin_lines), 1):
                    if a == t:
                        continue
                    a_s = a.decode("utf-8", "surrogateescape")
                    t_s = t.decode("utf-8", "surrogateescape")
                    here = [r for r in file_reverts if r["line"] == n and not r.get("used")]
                    ok, used = explained_by_revert(a_s, t_s, here)
                    if ok:
                        for r in used:
                            r["used"] = True
                        totals["reverts_explained"] += len(used)
                    else:
                        differing.append((a_s, t_s))
                        problems.append(f"line {n}:\n    adopt: {a_s.strip()[:200]}\n    twin:  {t_s.strip()[:200]}")
            defect = SCRIPT_DEFECTS.get((pid, str(rel)))
            if problems and defect and all(explained_by_defect(a, t) for a, t in differing):
                totals["files_script_defect"] += 1
                print(f"{pid}/{rel}: adopt converts {len(differing)} lines the script left dead — script defect: {defect}")
                continue
            if problems:
                totals["files_unexplained"] += 1
                print(f"{pid}/{rel}: UNEXPLAINED")
                for p in problems:
                    print("  " + p)
                    unexplained.append((pid, str(rel), p))
            else:
                totals["files_explained"] += 1
                print(f"{pid}/{rel}: differs only by the script's {sum(1 for r in file_reverts if r.get('used'))} check-driven reverts")
    print()
    print("summary:", json.dumps(totals))
    if unexplained:
        print(f"{len(unexplained)} unexplained differences")
        sys.exit(1)


if __name__ == "__main__":
    main()
