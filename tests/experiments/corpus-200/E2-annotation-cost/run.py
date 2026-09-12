#!/usr/bin/env python3
"""E2: mechanical annotation ramp on every corpus paper, with compiler-guided revert of leaks.
Per paper: work/<id>/ holds the twin; out/<id>.json holds every measurement; out/<id>.check.json the
final `xtex check --json`. Usage: run.py [paper-id ...] (default: whole manifest)."""
import csv, difflib, json, os, re, shutil, subprocess, sys, time
from pathlib import Path

HERE = Path(__file__).resolve().parent
CORPUS = HERE.parent
XTEX = Path(os.environ.get("XTEX", str(Path(__file__).resolve().parents[1] / "bin" / "xtex-d0e0caa")))
WORK = HERE / os.environ.get("WORK_DIR", "work"); OUT = HERE / os.environ.get("OUT_DIR", "out")

CITE_CMDS = ("citep", "citet", "cite", "textcite", "parencite")          # default set (grammar.md §4)
OTHER_CITE = ("citealp", "citealt", "citeauthor", "citeyear", "citeyearpar", "citenum", "citeal", "cites", "footcite", "autocite", "fullcite", "citetitle", "citeurl", "citealp*", "citep*", "citet*", "citeauthor*")
OTHER_REF = ("cref", "Cref", "eqref", "autoref", "pageref", "nameref", "vref", "Vref", "crefrange", "Crefrange", "hyperref")
BIBKEY = r"[A-Za-z0-9][A-Za-z0-9_:.+/-]*"
IDENT = r"[A-Za-z][A-Za-z0-9_:.-]*"
RX_CITE = re.compile(r"\\(" + "|".join(CITE_CMDS) + r")\{(" + BIBKEY + r"(?:," + BIBKEY + r")*)\}")
RX_LABEL = re.compile(r"\\label\{(" + IDENT + r")\}")
RX_REF = re.compile(r"\\ref\{(" + IDENT + r")\}")
RX_INPUT = re.compile(r"\\input\{([^{}]+)\}")
RX_ANY_CITE = re.compile(r"\\(" + "|".join(CITE_CMDS) + r")(\*?)(\[[^\]]*\])*\{([^{}]*)\}")
RX_OTHER_CITE = re.compile(r"\\(" + "|".join(re.escape(c) for c in OTHER_CITE) + r")\b")
RX_ANY_LABEL = re.compile(r"\\label\{([^{}]*)\}")
RX_ANY_REF = re.compile(r"\\ref\{([^{}]*)\}")
RX_OTHER_REF = re.compile(r"\\(" + "|".join(OTHER_REF) + r")\b")
RX_FLOAT = re.compile(r"\\begin\{(figure|table|figure\*|table\*|subfigure|wrapfigure|sidewaysfigure|sidewaystable)\}")
RX_LEAK = re.compile(r"@(cite|citep|citet|textcite|parencite|id|ref|import)\(([^)\n]*)\)")
MATH_ENVS = {"equation", "align", "gather", "multline", "eqnarray", "flalign", "alignat", "displaymath", "math", "subequations", "IEEEeqnarray"}
DEAD_ENVS = {"comment", "verbatim", "verbatimtab", "Verbatim", "listing", "lstlisting", "minted", "BVerbatim", "LVerbatim", "alltt"}

def live_spans(s):
    """Spans of s that the ramp may touch: outside comments, dead/verbatim envs, \\iffalse, and math."""
    spans = []; i = 0; n = len(s); start = 0
    def close(a, b):
        if b > a: spans.append((a, b))
    while i < n:
        c = s[i]
        if c == "\\":
            m = re.match(r"\\([A-Za-z]+)", s[i:])
            if m:
                w = m.group(1); j = i + m.end()
                if w == "verb":
                    k = j
                    if k < n and s[k] == "*": k += 1
                    if k < n and s[k] not in "\n":
                        d = s[k]; e = s.find(d, k + 1)
                        if e < 0: e = n
                        close(start, i); i = e + 1; start = i; continue
                    i = j; continue
                if w == "begin":
                    m2 = re.match(r"\s*\{([A-Za-z*]+)\}", s[j:])
                    if m2:
                        env = m2.group(1)
                        if env in DEAD_ENVS or env.rstrip("*") in MATH_ENVS:
                            e = s.find("\\end{" + env + "}", j)
                            e = n if e < 0 else e + len("\\end{" + env + "}")
                            hdr = j + m2.end()
                            if env.rstrip("*") in MATH_ENVS:
                                # header slot (grammar.md §4): whitespace with <=2 line endings, then \label -> stays live
                                m3 = re.match(r"(\s*)\\label\{" + IDENT + r"\}", s[hdr:])
                                if m3 and m3.group(1).count("\n") <= 2 and len(m3.group(1)) <= 256:
                                    close(start, hdr + m3.end()); i = e; start = i; continue
                            close(start, i); i = e; start = i; continue
                    i = j; continue
                if w == "iffalse":
                    e = s.find("\\fi", j); e = n if e < 0 else e + 3
                    close(start, i); i = e; start = i; continue
                i = j; continue
            if i + 1 < n and s[i + 1] in "[(":
                closer = "\\]" if s[i + 1] == "[" else "\\)"
                e = s.find(closer, i + 2); e = n if e < 0 else e + 2
                close(start, i); i = e; start = i; continue
            i += 2; continue   # control symbol: \% \$ \\ \{ ...
        if c == "%":
            e = s.find("\n", i); e = n if e < 0 else e
            close(start, i); i = e; start = i; continue
        if c == "$":
            if s.startswith("$$", i):
                e = s.find("$$", i + 2); e = n if e < 0 else e + 2
            else:
                e = i + 1
                while e < n:                       # next unescaped $
                    if s[e] == "\\": e += 2; continue
                    if s[e] == "$": break
                    e += 1
                e = n if e >= n else e + 1
            close(start, i); i = e; start = i; continue
        i += 1
    close(start, n)
    return spans

def convert_text(s, root_dir, is_root, paper_dir, stats, imports):
    out = []; pos = 0
    for a, b in live_spans(s):
        out.append(s[pos:a]); seg = s[a:b]
        # inventory of what is present in live text
        for m in RX_ANY_CITE.finditer(seg):
            stats["cite_cmds_live"] += 1
            keys = [k.strip() for k in m.group(4).split(",")]
            stats["cite_keys_live"] += len(keys)
            if m.group(2) == "*" or m.group(3): stats["cite_with_optarg_or_star"] += 1
            elif not RX_CITE.fullmatch(m.group(0)): stats["cite_keys_outside_grammar"] += 1
        stats["other_cite_cmds"] += len(RX_OTHER_CITE.findall(seg))
        for m in RX_ANY_LABEL.finditer(seg):
            stats["labels_live"] += 1
            if not RX_LABEL.fullmatch(m.group(0)): stats["labels_outside_grammar"] += 1
        for m in RX_ANY_REF.finditer(seg):
            stats["refs_live"] += 1
            if not RX_REF.fullmatch(m.group(0)): stats["refs_outside_grammar"] += 1
        stats["other_ref_cmds"] += len(RX_OTHER_REF.findall(seg))
        stats["float_envs"] += len(RX_FLOAT.findall(seg))
        # conversions
        def cite_sub(m):
            stats["cite_cmds_annotated"] += 1; stats["cite_keys_annotated"] += m.group(2).count(",") + 1
            return f"@{m.group(1)}({m.group(2)})"
        seg = RX_CITE.sub(cite_sub, seg)
        def label_sub(m):
            stats["ids_annotated"] += 1; return f"@id({m.group(1)})"
        seg = RX_LABEL.sub(label_sub, seg)
        def ref_sub(m):
            stats["refs_annotated"] += 1; return f"@ref({m.group(1)})"
        seg = RX_REF.sub(ref_sub, seg)
        if is_root:
            def input_sub(m):
                p = m.group(1).strip()
                stats["inputs_root"] += 1
                if any(ch in p for ch in "\\#$"): stats["inputs_left"] += 1; return m.group(0)
                cand = p if p.endswith(".tex") else p + ".tex"
                if (paper_dir / cand).is_file():
                    stats["inputs_imported"] += 1; imports.append(cand)
                    return f'@import("{cand[:-4]}.xtex")'
                stats["inputs_left"] += 1; return m.group(0)
            seg = RX_INPUT.sub(input_sub, seg)
        stats["inputs_nested_or_include_left"] += len(re.findall(r"\\include\{", seg)) + (0 if is_root else len(RX_INPUT.findall(seg)))
        out.append(seg); pos = b
    out.append(s[pos:])
    return "".join(out)

def run(cmd, cwd, timeout=120):
    try:
        p = subprocess.run(cmd, cwd=cwd, capture_output=True, text=True, timeout=timeout)
        return p.returncode, p.stdout, p.stderr
    except subprocess.TimeoutExpired:
        return "timeout", "", ""

def unleak(token, original_line=""):
    m = RX_LEAK.fullmatch(token); kind, arg = m.group(1), m.group(2)
    if kind == "import":
        stem = arg.strip('"')[:-5]
        for m2 in RX_INPUT.finditer(original_line):
            p = m2.group(1).strip()
            if p == stem or p == stem + ".tex": return m2.group(0)
        return "\\input{" + stem + "}"
    if kind == "id": return "\\label{" + arg + "}"
    if kind == "ref": return "\\ref{" + arg + "}"
    return "\\" + kind + "{" + arg + "}"

def process(pid, main_rel):
    src = CORPUS / "papers" / pid; wd = WORK / pid
    if wd.exists(): shutil.rmtree(wd)
    shutil.copytree(src, wd, symlinks=False)
    stats = {k: 0 for k in ["cite_cmds_live", "cite_keys_live", "cite_with_optarg_or_star", "cite_keys_outside_grammar", "other_cite_cmds",
                            "labels_live", "labels_outside_grammar", "refs_live", "refs_outside_grammar", "other_ref_cmds", "float_envs",
                            "cite_cmds_annotated", "cite_keys_annotated", "ids_annotated", "refs_annotated",
                            "inputs_root", "inputs_imported", "inputs_left", "inputs_nested_or_include_left"]}
    converted = {}   # rel .tex -> (original text, twin text)
    per_file = {}    # rel -> stats of that file alone
    imports = []
    main = Path(main_rel)
    orig = (src / main).read_text(encoding="utf-8", errors="surrogateescape")
    per_file[main] = dict(stats); converted[main] = (orig, convert_text(orig, src, True, src, per_file[main], imports))
    for imp in imports:
        rel = Path(imp)
        if rel in converted: continue
        o = (src / rel).read_text(encoding="utf-8", errors="surrogateescape")
        per_file[rel] = dict(stats); converted[rel] = (o, convert_text(o, src, False, src, per_file[rel], imports))
    def aggregate():
        for k in per_file[main]: stats[k] = sum(per_file[r][k] for r in converted)
    aggregate()
    for rel, (o, t) in converted.items():
        (wd / rel).unlink(); (wd / rel).with_suffix(".xtex").write_text(t, encoding="utf-8", errors="surrogateescape")
    root = main.with_suffix(".xtex")
    all_tex = "\n".join((p.read_text(encoding="utf-8", errors="surrogateescape")) for p in src.rglob("*.tex"))
    all_bib = "\n".join((p.read_text(encoding="utf-8", errors="surrogateescape")) for p in src.rglob("*.bib"))
    def label_defined(name): return len(re.findall(r"\\label\{\s*" + re.escape(name) + r"\s*\}", all_tex)) + len(re.findall(r"label\s*=\s*\{?" + re.escape(name) + r"\}?[,\]]", all_tex))
    def key_in_bib(key): return re.search(r"@\w+\s*[{(]\s*" + re.escape(key) + r"\s*,", all_bib) is not None
    # snapshot of the UNREPAIRED twin (first pass, before any revert): chars, first check, first emission attempt
    unrepaired = dict(chars_after=sum(len((wd / r).with_suffix(".xtex").read_text(encoding="utf-8", errors="surrogateescape")) for r in converted),
                      chars_before=sum(len(converted[r][0]) for r in converted), converted_files=len(converted))
    ccode0, cout0, cerr0 = run([str(XTEX), "check", "--json", str(root)], wd)
    try: check0 = json.loads(cout0.strip().splitlines()[-1])
    except Exception: check0 = {}
    unrepaired.update(check_exit=ccode0, coverage=check0.get("coverage"), bibliography=check0.get("bibliography"),
                      errors=sum(1 for d in check0.get("diagnostics", []) if d.get("severity") == "error"),
                      advisories=sum(1 for d in check0.get("diagnostics", []) if d.get("severity") != "error"))
    bcode0, _, _ = run([str(XTEX), "build", str(root)], wd)
    unrepaired["build_exit"] = bcode0; unrepaired["build_emitted"] = (wd / "build" / main).exists()
    if (wd / "build").exists(): shutil.rmtree(wd / "build")
    (OUT / f"{pid}.unrepaired.check.json").write_text(json.dumps(check0, indent=1))
    # compiler-guided neutralization: every hard error is classified (real defect vs compiler-unreachable) and its construct reverted
    defects = []
    for rnd in range(8):
        ccode, cout, cerr = run([str(XTEX), "check", "--json", str(root)], wd)
        try: check = json.loads(cout.strip().splitlines()[-1])
        except Exception: check = {"diagnostics": []}
        errs = [d for d in check.get("diagnostics", []) if d.get("severity") == "error"]
        if not errs: break
        reverted = 0
        for d in errs:
            f = wd / d["span"]["file"]; ln = d["span"]["line"] - 1; name = d.get("name", "")
            lines = f.read_text(encoding="utf-8", errors="surrogateescape").split("\n")
            line = lines[ln] if ln < len(lines) else ""
            rec = dict(code=d["code"], name=name, file=d["span"]["file"], line=ln + 1, message=d["message"])
            new = line
            if d["code"] == "XT1003":
                n_def = label_defined(name)
                rec["classification"] = "real: no \\label with this name anywhere in the sources" if n_def == 0 else f"compiler-unreachable: \\label{{{name}}} exists ({n_def}x) in a region the inventory does not read"
                new = line.replace(f"@ref({name})", f"\\ref{{{name}}}", 1)
            elif d["code"] == "XT1001":
                n_def = label_defined(name)
                rec["classification"] = f"real: \\label{{{name}}} appears {n_def}x in the sources" if n_def >= 2 else f"suspect: only {n_def} \\label in sources"
                new = line.replace(f"@id({name})", f"\\label{{{name}}}", 1)
            elif d["code"] == "XT1005":
                rec["classification"] = "real: key absent from every .bib in the paper" if not key_in_bib(name) else "suspect: key present in a .bib file"
                m = re.search(r"@(cite|citep|citet|textcite|parencite)\(([^)]*)\)", line)
                # revert the construct on this line that carries the key
                for m in re.finditer(r"@(cite|citep|citet|textcite|parencite)\(([^)]*)\)", line):
                    if name in m.group(2).split(","):
                        new = line[: m.start()] + "\\" + m.group(1) + "{" + m.group(2) + "}" + line[m.end():]; break
            elif d["code"] in ("XT1004", "XT1020"):
                m_t = re.search(r"target is (\w+)|is an? (\w+)$", d["message"]); tclass = (m_t.group(1) or m_t.group(2)) if m_t else "?"
                has_appendix = re.search(r"^[^%\n]*\\appendix\b", all_tex, re.M) is not None
                rec["classification"] = ("prefix-class mismatch on a real document" if d["code"] == "XT1004" else "prose-word mismatch on a real document") + f" [target classed as {tclass}]: `{name.split(':')[0]}:` prefix" + ("; the paper uses \\appendix" if has_appendix else "")
                new = line.replace(f"@ref({name})", f"\\ref{{{name}}}", 1)
            else:
                rec["classification"] = "unexpected code, not reverted"
            if new != line:
                lines[ln] = new; f.write_text("\n".join(lines), encoding="utf-8", errors="surrogateescape"); reverted += 1; rec["reverted"] = True
            else:
                rec["reverted"] = False
            if not any(x["code"] == rec["code"] and x["name"] == rec["name"] and x["file"] == rec["file"] and x["line"] == rec["line"] for x in defects):
                defects.append(rec)
        if not reverted: break
    stats["defects_reverted"] = sum(1 for x in defects if x["reverted"])
    for x in defects:
        if not x["reverted"]: continue
        if x["code"] in ("XT1003", "XT1004"): stats["refs_annotated"] -= 1
        elif x["code"] == "XT1001": stats["ids_annotated"] -= 1
        elif x["code"] == "XT1005": stats["cite_cmds_annotated"] -= 1
    # compiler-guided revert of leaks
    leaks = []; unimported = []
    for rnd in range(6):
        if (wd / "build").exists(): shutil.rmtree(wd / "build")
        bcode, bout, berr = run([str(XTEX), "build", str(root)], wd)
        found = 0
        for rel in converted:
            em = wd / "build" / rel
            if not em.exists(): continue
            elines = em.read_text(encoding="utf-8", errors="surrogateescape").split("\n")
            olines = converted[rel][0].split("\n")
            xpath = (wd / rel).with_suffix(".xtex"); xtext = xpath.read_text(encoding="utf-8", errors="surrogateescape")
            xlines = xtext.split("\n"); changed = False
            for ln, line in enumerate(elines):
                for m in RX_LEAK.finditer(line):
                    tok = m.group(0)
                    if ln < len(olines) and tok in olines[ln]: continue   # the original really contains it
                    if ln < len(xlines) and tok in xlines[ln]:
                        ctx = olines[ln] if ln < len(olines) else ""
                        rep = unleak(tok, ctx)
                        xlines[ln] = xlines[ln].replace(tok, rep, 1); changed = True; found += 1
                        if tok.startswith("@import("): unimported.append(tok[9:-2])
                        pre = ctx[: ctx.find(rep)] if rep in ctx else ctx
                        wrapper = re.findall(r"\\([A-Za-z]+\*?)(?:\[[^\]]*\])?\{[^{}]*$", pre)
                        leaks.append(dict(file=str(rel), line=ln + 1, token=tok, wrapper=wrapper[-1] if wrapper else "?", context=ctx.strip()[:160]))
            if changed: xpath.write_text("\n".join(xlines), encoding="utf-8", errors="surrogateescape")
        if not found: break
    for imp in unimported:
        rel = Path(imp[:-5] + ".tex")
        still = any(f'@import("{imp}")' in (wd / r).with_suffix(".xtex").read_text(encoding="utf-8", errors="surrogateescape") for r in converted if (wd / r).with_suffix(".xtex").exists())
        if not still and rel in converted:
            (wd / rel).with_suffix(".xtex").unlink(missing_ok=True); shutil.copy(src / rel, wd / rel); del converted[rel]
            aggregate(); stats["inputs_imported"] -= 1; stats["inputs_left"] += 1
    stats["leaks_reverted"] = len(leaks); stats["revert_rounds"] = rnd
    for k, kind in (("cite_cmds_annotated", ("cite", "citep", "citet", "textcite", "parencite")), ("ids_annotated", ("id",)), ("refs_annotated", ("ref",))):
        stats[k] -= sum(1 for l in leaks if RX_LEAK.fullmatch(l["token"]).group(1) in kind)
    stats["cite_keys_annotated"] -= sum(RX_LEAK.fullmatch(l["token"]).group(2).count(",") + 1 for l in leaks if RX_LEAK.fullmatch(l["token"]).group(1) not in ("id", "ref", "import"))
    stats["import_leaks"] = len(unimported)
    # final check + build
    ccode, cout, cerr = run([str(XTEX), "check", "--json", str(root)], wd)
    try: check = json.loads(cout.strip().splitlines()[-1])
    except Exception: check = {"parse_error": cout[-500:] + cerr[-500:]}
    (OUT / f"{pid}.check.json").write_text(json.dumps(check, indent=1))
    if (wd / "build").exists(): shutil.rmtree(wd / "build")
    bcode, bout, berr = run([str(XTEX), "build", str(root)], wd)
    files = []
    chars_before = chars_after = 0
    for rel, (o, t) in converted.items():
        twin = (wd / rel).with_suffix(".xtex").read_text(encoding="utf-8", errors="surrogateescape")
        chars_before += len(o); chars_after += len(twin)
        em = wd / "build" / rel
        rec = dict(file=str(rel), chars_before=len(o), chars_after=len(twin), emitted=em.exists(), identity="missing")
        if em.exists():
            e = em.read_text(encoding="utf-8", errors="surrogateescape")
            if e == o: rec["identity"] = "identical"
            else:
                ol, el = o.split("\n"), e.split("\n")
                import_only = len(ol) == len(el) and all(a == b or (RX_INPUT.search(a) and re.sub(r"\\input\{([^{}]+?)(\.tex)?\}", r"\\input{\1.tex}", a) == b) for a, b in zip(ol, el))
                rec["identity"] = "import-ext-only" if import_only else "other"
                rec["lines_differing"] = sum(1 for a, b in zip(ol, el) if a != b) + abs(len(ol) - len(el))
                if not import_only:
                    (OUT / f"{pid}__{str(rel).replace('/', '_')}.diff").write_text("".join(difflib.unified_diff(o.splitlines(True), e.splitlines(True), "original", "emitted")))
        files.append(rec)
    unrepaired["delta_pct"] = (100.0 * (unrepaired["chars_after"] - unrepaired["chars_before"]) / unrepaired["chars_before"]) if unrepaired["chars_before"] else 0.0
    res = dict(paper=pid, main=main_rel, files=files, converted_files=len(converted), chars_before=chars_before, chars_after=chars_after,
               delta_pct=(100.0 * (chars_after - chars_before) / chars_before) if chars_before else 0.0, unrepaired=unrepaired,
               stats=stats, leaks=leaks, defects=defects, check_exit=ccode, build_exit=bcode, coverage=check.get("coverage"), bibliography=check.get("bibliography"),
               diagnostics=check.get("diagnostics", []), check_stderr=cerr[-300:], build_stderr=berr[-300:])
    (OUT / f"{pid}.json").write_text(json.dumps(res, indent=1))
    errs = [d["code"] for d in res["diagnostics"]]
    print(f"{pid}: files={len(converted)} delta={res['delta_pct']:+.3f}% cites={stats['cite_cmds_annotated']} ids={stats['ids_annotated']} refs={stats['refs_annotated']} leaks={len(leaks)} defects={[(d['code'],d['name']) for d in defects]} check={ccode} diags={errs[:8]} cov={check.get('coverage')} bib={check.get('bibliography')} ident={[f['identity'] for f in files]}", flush=True)

def main():
    OUT.mkdir(exist_ok=True); WORK.mkdir(exist_ok=True)
    papers = {r["id"]: r for r in csv.DictReader(open(CORPUS / "manifest.csv"))}
    ids = sys.argv[1:] or list(papers)
    for pid in ids:
        try: process(pid, papers[pid]["main_file"])
        except Exception as e:
            import traceback; traceback.print_exc()
            (OUT / f"{pid}.json").write_text(json.dumps(dict(paper=pid, failed=repr(e))))
            print(f"{pid}: FAILED {e!r}", flush=True)

if __name__ == "__main__":
    main()
