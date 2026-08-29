#!/usr/bin/env python3
"""Ground truth: what real BibTeX 0.99e does with a .bib file.

Runs bibtex over a minimal .aux that cites everything, and records whether it
reported an error. This is the reference the survey is measured against: the
question is never "is this file good BibTeX" in the abstract, it is "does the
program that consumes it accept it".
"""
import os, re, shutil, subprocess, sys, tempfile

def run(bib):
    d = tempfile.mkdtemp()
    try:
        shutil.copy(bib, os.path.join(d, "refs.bib"))
        with open(os.path.join(d, "x.aux"), "w") as f:
            f.write("\\relax\n\\citation{*}\n\\bibstyle{plain}\n\\bibdata{refs}\n")
        r = subprocess.run(["bibtex", "x"], cwd=d, capture_output=True, text=True)
        out = r.stdout + r.stderr
        errors = [l for l in out.splitlines()
                  if re.search(r"^(I was expecting|Sorry|Illegal|You're missing|Repeated entry"
                               r"|A bad cross reference|I found no|Warning--|.*---line \d+)", l)
                  or "error message" in l]
        hard = [l for l in errors if not l.startswith("Warning--")]
        return r.returncode, hard, out
    finally:
        shutil.rmtree(d, ignore_errors=True)

for bib in sys.argv[1:]:
    code, hard, out = run(bib)
    verdict = "REJECT" if hard else "accept"
    print(f"{verdict:7} exit={code}  {os.path.basename(bib)}")
    for l in hard[:2]:
        print(f"         {l.strip()[:100]}")
