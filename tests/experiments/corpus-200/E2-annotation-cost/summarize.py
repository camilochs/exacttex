#!/usr/bin/env python3
"""E2 tables from out/<id>.json. Prints markdown."""
import csv, json, statistics as st
from collections import Counter
from pathlib import Path
import os
HERE = Path(__file__).resolve().parent; CORPUS = HERE.parent; OUTD = HERE / os.environ.get("OUT_DIR", "out")
man = {r["id"]: r for r in csv.DictReader(open(CORPUS / "manifest.csv"))}
R = [json.loads(p.read_text()) for p in sorted(OUTD.glob("*.json")) if not p.name.endswith(".check.json")]
ok = [r for r in R if not r.get("failed")]
fail = [r for r in R if r.get("failed")]
S = lambda k: [r["stats"][k] for r in ok]
def q(v, f="{:.3f}"): return f.format(v)
deltas = [r["delta_pct"] for r in ok]
print("## Papers\n")
print(f"{len(R)} papers processed, {len(ok)} converted, {len(fail)} failed ({', '.join(r['paper'] for r in fail)}).\n")
print("## Papers and files converted\n")
print("| Statistic | Value |\n|---|---|")
print(f"| Papers | {len(ok)} |")
print(f"| Converted files (root + root-level `\\input`) | {sum(r['converted_files'] for r in ok)} |")
print()
U = [r for r in ok if r.get("unrepaired")]
if U:
    ud = [r["unrepaired"]["delta_pct"] for r in U]
    print("### The UNREPAIRED twin (first pass: every convertible construct annotated, nothing reverted)\n")
    print("| Statistic | Value |\n|---|---|")
    print(f"| Converted files (before any un-import) | {sum(r['unrepaired'].get('converted_files', 0) for r in U)} |")
    print(f"| `xtex check` exit 0 / exit 1 | {sum(1 for r in U if r['unrepaired']['check_exit']==0)} / {sum(1 for r in U if r['unrepaired']['check_exit']==1)} |")
    print(f"| Error diagnostics (sum) | {sum(r['unrepaired']['errors'] for r in U)} |")
    uc = [r["unrepaired"]["coverage"] for r in U if r["unrepaired"].get("coverage") is not None]
    print(f"| Coverage per paper: median / min / max | {100*st.median(uc):.1f}% / {100*min(uc):.1f}% / {100*max(uc):.1f}% |")
    print(f"| `xtex build` emitted output | {sum(1 for r in U if r['unrepaired']['build_emitted'])} / {len(U)} (build refuses on hard errors, so byte identity is measured on the repaired twin) |")
    print()
print("## What the ramp annotated and what it left transported (sums over the corpus)\n")
keys = [("cite_cmds_live", "citation commands of the default set in live text"), ("cite_cmds_annotated", "  → annotated (`@cite…`)"), ("cite_keys_annotated", "  → keys checked"),
        ("cite_with_optarg_or_star", "  → left: optional argument or starred form"), ("cite_keys_outside_grammar", "  → left: key list with whitespace or characters outside `bibkey`"),
        ("other_cite_cmds", "citation commands outside the default set (`\\citealp`, `\\citeauthor`, …), left"),
        ("labels_live", "`\\label` in live text"), ("ids_annotated", "  → annotated (`@id`)"), ("labels_outside_grammar", "  → left: identifier outside `ident`"),
        ("refs_live", "plain `\\ref` in live text"), ("refs_annotated", "  → annotated (`@ref`)"), ("refs_outside_grammar", "  → left: identifier outside `ident`"),
        ("other_ref_cmds", "`\\cref`/`\\Cref`/`\\eqref`/`\\autoref`/… (no `@` form), left"), ("float_envs", "figure/table environments, left"),
        ("inputs_root", "`\\input` in the root file"), ("inputs_imported", "  → became `@import`"), ("inputs_left", "  → left (target not found / computed)"),
        ("inputs_nested_or_include_left", "nested `\\input` and `\\include`, left"), ("leaks_reverted", "constructs reverted because they leaked into the emission (opaque region)"),
        ("defects_reverted", "constructs reverted because the check flagged them (see defects)")]
print("| Quantity | Sum | Median per paper | Max |\n|---|---|---|---|")
for k, label in keys:
    v = S(k); print(f"| {label} | {sum(v)} | {st.median(v):g} | {max(v)} |")
print()
def enclosing(paper, file, line, token):
    """Innermost unclosed brace group at the leak position in the ORIGINAL file, and the command that opened it."""
    text = (CORPUS / "papers" / paper / file).read_text(encoding="utf-8", errors="surrogateescape")
    lines = text.split("\n"); latex = {"id": "\\label{", "ref": "\\ref{"}.get(token.split("(")[0][1:], "\\" + token.split("(")[0][1:] + "{")
    col = lines[line - 1].find(latex); pos = sum(len(l) + 1 for l in lines[: line - 1]) + max(col, 0)
    depth = 0; bdepth = 0; i = pos - 1
    while i >= 0:
        c = text[i]; esc = i > 0 and text[i - 1] == "\\"
        if c == "}" and not esc: depth += 1
        elif c == "]" and not esc: bdepth += 1
        elif c == "[" and not esc and depth == 0:
            if bdepth == 0:
                m = re.search(r"\\([A-Za-z@]+\*?)\s*((?:\[[^\]]*\]|\{[^{}]*\})*)\s*$", text[max(0, i - 200): i])
                return ("[optional arg of " + ("\\" + m.group(1) + ("{" + m.group(2).strip("{}") + "}" if m.group(1) == "begin" else "")) + "]") if m else "[optional arg]"
            bdepth -= 1
        elif c == "{" and not esc:
            if depth == 0:
                m = re.search(r"\\([A-Za-z@]+\*?)\s*((?:\[[^\]]*\]|\{[^{}]*\})*)\s*$", text[max(0, i - 200): i])
                return ("\\" + m.group(1)) if m else ("{ (bare group)" if i == 0 or text[i-1] not in "]}" else "{ after " + text[max(0,i-20):i].strip()[-20:])
            depth -= 1
        i -= 1
    return "(top level)"
import re
for r in ok:
    for l in r["leaks"]: l["encl"] = enclosing(r["paper"], l["file"], l["line"], l["token"])
print("## Leaks (constructs the compiler treated as opaque), by the innermost enclosing command in the original\n")
wr = Counter(l["encl"] for r in ok for l in r["leaks"])
print("| Wrapper | Leaks |\n|---|---|")
for w, c in wr.most_common(): print(f"| `{w}` | {c} |")
tot_constructs = sum(S("cite_cmds_annotated")) + sum(S("ids_annotated")) + sum(S("refs_annotated")) + sum(S("leaks_reverted")) + sum(S("defects_reverted"))
print(f"\nLeak rate: {sum(S('leaks_reverted'))} leaks (incl. {sum(r['stats'].get('import_leaks', 0) for r in ok)} `@import`) / {tot_constructs} first-pass constructs (annotated + leaked + reverted for a diagnostic) = {100*sum(S('leaks_reverted'))/tot_constructs:.2f}%.\n")
print("Examples (one per enclosing command, first occurrence):\n")
seen = set()
for r in ok:
    for l in r["leaks"]:
        if l["encl"] in seen: continue
        seen.add(l["encl"]); print(f"- {r['paper']} `{l['file']}:{l['line']}` `{l['token']}` inside `{l['encl']}` — `{l['context'][:100]}`")
print()
import sys; sys.path.insert(0, str(HERE)); from run import live_spans, MATH_ENVS
def label_region(paper, name):
    """Where the \\label{name} sits in the ORIGINAL sources: 'math body', 'listing/verbatim', or 'command or environment argument'."""
    kinds = set()
    for p in (CORPUS / "papers" / paper).rglob("*.tex"):
        t = p.read_text(encoding="utf-8", errors="surrogateescape")
        for m in re.finditer(r"\\label\{\s*" + re.escape(name) + r"\s*\}", t):
            live = any(a <= m.start() < b for a, b in live_spans(t))
            if live: kinds.add("command or environment argument"); continue
            before = t[: m.start()]
            env = re.findall(r"\\begin\{([A-Za-z*]+)\}", before); ends = re.findall(r"\\end\{([A-Za-z*]+)\}", before)
            opened = [e for e in env if env.count(e) > ends.count(e)]
            if any(e.rstrip("*") in MATH_ENVS for e in opened) or before.count("$") % 2 == 1 or before.rfind("\\[") > before.rfind("\\]"): kinds.add("math body")
            elif any(e in ("lstlisting", "verbatim", "Verbatim", "minted", "comment") for e in opened): kinds.add("listing/verbatim/comment env")
            else: kinds.add("other dead region (comment?)")
    return ", ".join(sorted(kinds)) or "?"
for r in ok:
    for d in r["defects"]:
        if d["code"] == "XT1003" and d["classification"].startswith("compiler-unreachable"): d["classification"] += " [" + label_region(r["paper"], d["name"]) + "]"
print("## Diagnostics raised on the first pass, classified against the sources\n")
def ckey(d):
    c = d["classification"]; m = re.search(r"\[([^\]]*)\]", c)
    return (d["code"], c.split(":")[0].split(" [")[0] + (" [" + m.group(1) + "]" if m and (c.startswith("compiler-unreachable") or "mismatch" in c) else ""))
cls = Counter(ckey(d) for r in ok for d in r["defects"])
print("| Code | Classification | Count | Papers |\n|---|---|---|---|")
for key, n in sorted(cls.items()):
    ps = sorted({r["paper"] for r in ok for d in r["defects"] if ckey(d) == key})
    print(f"| {key[0]} | {key[1]} | {n} | {len(ps)} |")
print("\nEvery record:\n")
for r in ok:
    for d in r["defects"]: print(f"- {r['paper']} `{d['file']}:{d['line']}` {d['code']} `{d['name']}` — {d['classification']}")
print()
print("## Final twin: check result, bibliography, coverage, emission identity\n")
print("| Quantity | Value |\n|---|---|")
print(f"| Final `xtex check` exit 0 | {sum(1 for r in ok if r['check_exit'] == 0)} / {len(ok)} |")
print(f"| Diagnostics on the final twin (should be 0 errors; advisories listed) | " + ", ".join(f"{c}: {n}" for c, n in Counter(d['code'] for r in ok for d in r['diagnostics']).items()) + " |")
bibs = Counter((r["bibliography"] or {}).get("state", "?") + ("" if (r["bibliography"] or {}).get("state") == "complete" else f" ({(r['bibliography'] or {}).get('reason', '')})") for r in ok)
print(f"| Bibliography state | " + ", ".join(f"{k}: {n}" for k, n in bibs.most_common()) + " |")
cov = [r["coverage"] for r in ok if r["coverage"] is not None]
print(f"| Coverage per paper: median / min / max | {100*st.median(cov):.1f}% / {100*min(cov):.1f}% / {100*max(cov):.1f}% |")
opaque = [r["paper"] for r in ok if (r["coverage"] or 0) == 0]
print(f"| Papers whose root is entirely opaque (coverage 0, every construct leaked and was reverted) | {len(opaque)} ({', '.join(opaque)}) |")
print(f"| Root-level `\\input` reverted because the `@import` leaked (opaque region) | {sum(r['stats'].get('import_leaks', 0) for r in ok)} |")
idc = Counter(f["identity"] for r in ok for f in r["files"])
print(f"| Emitted files vs original | " + ", ".join(f"{k}: {n}" for k, n in idc.most_common()) + " |")
print(f"| Papers whose every emitted file is identical or import-ext-only | {sum(1 for r in ok if all(f['identity'] in ('identical','import-ext-only') for f in r['files']))} / {len(ok)} |")
print()
print("## Per paper\n")
print("| Paper | cat | files | chars | Δ% | cites | keys | ids | refs | leaks | defects (first pass) | bib | cov % | emission |\n|---|---|---|---|---|---|---|---|---|---|---|---|---|---|")
for r in ok:
    s = r["stats"]; b = (r["bibliography"] or {}); bs = "complete" if b.get("state") == "complete" else "unavail"
    dd = ", ".join(sorted({d["code"] + ("" if d["classification"].startswith("real") else "*") for d in r["defects"]})) or "—"
    em = "identical" if all(f["identity"] == "identical" for f in r["files"]) else ("import-ext" if all(f["identity"] in ("identical", "import-ext-only") for f in r["files"]) else "DIFFERS")
    print(f"| {r['paper']} | {man[r['paper']]['category']} | {r['converted_files']} | {r['chars_before']:,} | {r['delta_pct']:+.3f} | {s['cite_cmds_annotated']} | {s['cite_keys_annotated']} | {s['ids_annotated']} | {s['refs_annotated']} | {s['leaks_reverted']} | {dd} | {bs} | {100*(r['coverage'] or 0):.1f} | {em} |")
print("\n`*` = classified as not a real defect (compiler-unreachable label or suspect); see the record list.")
