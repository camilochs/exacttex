#!/usr/bin/env python3
"""E3: plant one Class-A defect per twin copy, run xtex / texlab / tectonic, record detection.
Usage: run.py [--no-tectonic] [paper-id ...]. Outputs out/results.csv and out/<paper>/<class>/ raw logs."""
import csv, json, os, re, resource, shutil, signal, subprocess, sys, tempfile, time
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

HERE = Path(__file__).resolve().parent
CORPUS = HERE.parent
E2 = CORPUS / "E2-annotation-cost"
XTEX = Path(os.environ.get("XTEX", str(Path(__file__).resolve().parents[1] / "bin" / "xtex-d0e0caa")))
WORK = HERE / os.environ.get("WORK_DIR", "work"); OUT = HERE / os.environ.get("OUT_DIR", "out")
TEXLAB_WAIT = int(os.environ.get("TEXLAB_WAIT", "8000"))
TECTONIC_TIMEOUT = int(os.environ.get("TECTONIC_TIMEOUT", "300"))
CLASSES = ["A1-broken-ref", "A2-missing-cite", "A3-duplicate-id", "A4-wrong-class", "A5-missing-figure", "A6-invalid-unit"]
CLASSES_RUN = CLASSES
LINTERS = False     # --linters: as --xtex-only, but chktex and chklref run (they never had in the d0e0caa pass)
XTEX_ONLY = False   # --xtex-only: skip texlab and tectonic; their columns are copied from HERE/out/results.csv (the d0e0caa pass)
RX_SECTION = re.compile(r"^[^%\n]*\\(sub)?section\*?\{[^%\n]*$")   # no comment marker anywhere on the line (an appended construct after `%` would be dead text)
IMG_EXT = {".pdf", ".png", ".jpg", ".jpeg", ".eps"}

def sh(cmd, cwd, timeout):
    """A tool run in its own process group, killed as a group on timeout.

    Why the group: chklref drives pdflatex, and killing chklref alone
    leaves the engine running. On 2026-09-02 that left hundreds of
    pdflatex processes alive, each writing into its own copy of a paper,
    and filled the disk — the run then made more copies and cleaned none,
    because a class that never finishes never reaches its cleanup.
    """
    t = time.time()
    # A tool under test may write without bound: chklref left a 5.8 GB
    # `.chk` file on one paper and filled the disk before its timeout was
    # anywhere near. Every child therefore runs under a file-size limit as
    # well as a clock, and dies on the byte that crosses it.
    cap = int(os.environ.get("MAX_FILE_MB", "256")) * 1024 * 1024

    def limit():
        resource.setrlimit(resource.RLIMIT_FSIZE, (cap, cap))

    # The tool's output goes to a file, not a pipe: the size limit above then
    # bounds it too. Through a pipe it was unbounded — chklref repeated one
    # finding until the runner held 2.8 GB of it (2026-09-06).
    with tempfile.TemporaryFile(mode="w+", encoding="utf-8", errors="replace") as fh:
        proc = subprocess.Popen(cmd, cwd=cwd, stdout=fh, stderr=subprocess.STDOUT, start_new_session=True, preexec_fn=limit)
        try:
            code = proc.wait(timeout=timeout)
        except subprocess.TimeoutExpired:
            try:
                os.killpg(os.getpgid(proc.pid), signal.SIGKILL)
            except ProcessLookupError:
                pass
            proc.wait(); code = "timeout"
        fh.seek(0); out = fh.read()
    return code, out, time.time() - t


def free_gb(path=None):
    st = os.statvfs(str(path or HERE))
    return st.f_bavail * st.f_frsize / 1024**3


FLOOR_GB = float(os.environ.get("FLOOR_GB", "20"))


def check_room(where):
    """A campaign stops itself before it fills the machine. The floor is
    generous on purpose: a measurement is worth less than a working disk."""
    room = free_gb()
    if room < FLOOR_GB:
        raise SystemExit(f"stopping at {where}: {room:.1f} GB free, below the {FLOOR_GB:.0f} GB floor. Clean work-* and run again.")

def read(p): return p.read_text(encoding="utf-8", errors="surrogateescape")
def write(p, s): p.write_text(s, encoding="utf-8", errors="surrogateescape")

# ---------- tool wrappers, each returning a set of comparable diagnostic tuples ----------
def xtex_diags(root_dir, root):
    code, out, _ = sh([str(XTEX), "check", "--json", root], root_dir, 120)
    try: j = json.loads(out.strip().splitlines()[-1])
    except Exception: return code, set(), out[-400:]
    return code, {(d["code"], d.get("name") or "", d["span"]["file"], d["span"]["line"]) for d in j.get("diagnostics", []) if d.get("severity") == "error"}, ""

def linter_findings(root_dir, root, tool, raw=None):
    """chktex and chklref on the .tex twin: their own findings, as E4 reads
    them. chklref drives pdflatex, so it is given room and its aux files are
    cleaned; a tool that is not installed is recorded as absent, never as
    silent — a missing tool that reads as "found nothing" would flatter us."""
    if not shutil.which(tool):
        return None
    if tool == "chktex":
        code, out, _ = sh(["chktex", "-q", root], root_dir, 120)
        found = {l.strip() for l in out.splitlines() if l.startswith(("Warning", "Error"))}
        if raw: write(raw, f"exit={code}\nlines={out.count(chr(10))}\nunique_findings={len(found)}\n" + out[-20000:])
        return found
    code, out, _ = sh(["chklref", root], root_dir, 300)
    # Only the root's own PDF is the tool's leaving: a paper's figures often
    # sit as PDFs beside the root, and the clean base is copied into every
    # planted twin after this cleanup ran on it.
    own_pdf = root_dir / Path(root).with_suffix(".pdf").name
    found = {l.strip() for l in out.splitlines() if l.startswith("-- ")}
    if raw: write(raw, f"pdf={int(own_pdf.exists())}\nexit={code}\nlines={out.count(chr(10))}\nunique_findings={len(found)}\n" + out[-20000:])
    # chklref's own leavings, whatever the document is called: the engine's
    # aux files and its `.chk` transcript, which is the one that grows.
    for f in list(root_dir.glob("*.aux")) + list(root_dir.glob("*.chk")) + list(root_dir.glob("*.log")) + list(root_dir.glob("*.out")) + list(root_dir.glob("*.dvi")) + [own_pdf]:
        try:
            f.unlink()
        except OSError:
            pass
    return found


def texlab_diags(root_dir, root):
    code, out, _ = sh(["node", str(HERE / "texlab-probe.mjs"), str(root_dir), root, str(TEXLAB_WAIT)], root_dir, TEXLAB_WAIT / 1000 + 30)
    s = set()
    for line in out.splitlines():
        m = re.match(r"(ERROR|WARN|info) (.+?) L(\d+): (.*)", line)
        if m: s.add((m.group(1), m.group(2), int(m.group(3)), m.group(4)))
    return s

def tectonic_run(root_dir, root):
    outdir = root_dir / "_tectonic"; outdir.mkdir(exist_ok=True)
    code, out, secs = sh(["tectonic", "--chatter", "minimal", "--keep-logs", "-o", str(outdir), root], root_dir, TECTONIC_TIMEOUT)
    log = ""
    for lg in outdir.glob("*.log"):
        try: log += lg.read_text(encoding="utf-8", errors="replace")
        except Exception: pass
    warn = set(re.findall(r"LaTeX Warning: (?:Reference|Citation) `([^']*)' on page|LaTeX Warning: Label `([^']*)' multiply defined", log))
    names = {a or b for a, b in warn}
    return code, out, log, names, secs

# ---------- planting ----------
def find_line(lines, rx, start=0, exclude=()):
    dead = 0
    for i in range(len(lines)):
        if re.search(r"\\begin\{(lstlisting|verbatim|Verbatim|minted|comment)\}", lines[i]): dead += 1
        if re.search(r"\\end\{(lstlisting|verbatim|Verbatim|minted|comment)\}", lines[i]): dead = max(0, dead - 1); continue
        if i < start or i in exclude or dead: continue
        if rx.search(lines[i]): return i
    return None

def begin_document_line(lines):
    i = find_line(lines, re.compile(r"^[^%\n]*\\begin\{document\}"))
    return i if i is not None else -1

def plant(cls, xroot_dir, xroot, troot_dir, troot, converted):
    """Edit the twin (xtex form) and the LaTeX form identically. Returns (planted_name, [(file, line)], note) or (None, [], reason)."""
    files = [xroot] + [f for f in converted if f != xroot]   # root first
    for xf in files:
        tf = xf[:-5] + ".tex"
        xl = read(xroot_dir / xf).split("\n"); tl = read(troot_dir / tf).split("\n")
        if len(xl) != len(tl): return None, [], f"line count differs between twin and original for {xf}"
        bd = begin_document_line(xl)
        if cls == "A1-broken-ref":
            for i, line in enumerate(xl):
                m = re.search(r"@ref\(([A-Za-z][A-Za-z0-9_:.-]*)\)", line)
                if m and f"\\ref{{{m.group(1)}}}" in tl[i]:
                    I = m.group(1)
                    xl[i] = line.replace(f"@ref({I})", f"@ref({I}__ghost)", 1); tl[i] = tl[i].replace(f"\\ref{{{I}}}", f"\\ref{{{I}__ghost}}", 1)
                    write(xroot_dir / xf, "\n".join(xl)); write(troot_dir / tf, "\n".join(tl))
                    return I + "__ghost", [(xf, i + 1)], ""
        elif cls == "A2-missing-cite":
            for i, line in enumerate(xl):
                m = re.search(r"@(cite|citep|citet|textcite|parencite)\(([A-Za-z0-9][A-Za-z0-9_:.+/-]*)([,)])", line)
                if m and f"\\{m.group(1)}{{{m.group(2)}{m.group(3).replace(')', '}')}" in tl[i]:
                    K = m.group(2); cmd = m.group(1); tail = m.group(3)
                    xl[i] = line.replace(f"@{cmd}({K}{tail}", f"@{cmd}({K}__ghost{tail}", 1)
                    ttail = tail.replace(")", "}")
                    tl[i] = tl[i].replace(f"\\{cmd}{{{K}{ttail}", f"\\{cmd}{{{K}__ghost{ttail}", 1)
                    write(xroot_dir / xf, "\n".join(xl)); write(troot_dir / tf, "\n".join(tl))
                    return K + "__ghost", [(xf, i + 1)], ""
        elif cls == "A3-duplicate-id":
            for i, line in enumerate(xl):
                m = re.search(r"@id\(([A-Za-z][A-Za-z0-9_:.-]*)\)", line)
                if not m or f"\\label{{{m.group(1)}}}" not in tl[i]: continue
                I = m.group(1)
                j = find_line(xl, RX_SECTION, bd + 1, exclude={i})
                if j is None: break
                xl[j] += f" @id({I})"; tl[j] += f" \\label{{{I}}}"
                write(xroot_dir / xf, "\n".join(xl)); write(troot_dir / tf, "\n".join(tl))
                return I, [(xf, i + 1), (xf, j + 1)], ""
        elif cls == "A4-wrong-class":
            # the checked form: a \table(fig:x) typed block referenced by @ref(fig:x). Convert one table float
            # (table or table*), line-count preserving, and rename every reference to its identifier in both forms.
            NEW = "fig:planted_wrongclass"
            i = 0
            while i < len(xl):
                mb = re.match(r"^\s*\\begin\{table\*?\}(\[([htbpH!]*)\])?\s*$", xl[i])
                if not mb: i += 1; continue
                j = next((k for k in range(i + 1, len(xl)) if re.match(r"^\s*\\end\{table\*?\}\s*$", xl[k])), None)
                if j is None: i += 1; continue
                body = xl[i + 1: j]; tbody = tl[i + 1: j]
                if "\\begin{table" in "\n".join(body) or "\\label{" in "\n".join(body) or "@id(" not in "\n".join(body): i = j + 1; continue
                nocomment = lambda l: re.split(r"(?<!\\)%", l)[0]
                caps = [k for k, l in enumerate(body) if re.match(r"^\s*\\caption\{", l)]
                if len(caps) != 1: i = j + 1; continue
                ci = caps[0]
                # caption end line by brace balance from the \caption{ opening
                depth = 0; ce = None; started = False
                for k in range(ci, len(body)):
                    for ch in re.sub(r"\\[{}]", "", nocomment(body[k])):
                        if ch == "{": depth += 1; started = True
                        elif ch == "}": depth -= 1
                    if started and depth == 0: ce = k; break
                if ce is None: i = j + 1; continue
                ids = [(k, m) for k in range(ce, len(body)) for m in [re.search(r"@id\(([^)]+)\)", body[k])] if m]
                if len(ids) != 1 or len(re.findall(r"@id\(", "\n".join(body))) != 1: i = j + 1; continue
                li, mid = ids[0]; orig_id = mid.group(1)
                if li == ce:
                    tail = body[ce][mid.end():]
                    if tail.strip(): i = j + 1; continue
                if li > ce and not re.fullmatch(r"\s*@id\([^)]+\)\s*", body[li]): i = j + 1; continue
                if tbody[ci].strip() != body[ci].strip() and li != ci: i = j + 1; continue
                placement = f" placement = {mb.group(2)}" if mb.group(2) else ""   # `[]` (empty) gives no placement field
                nx = list(xl); nt = list(tl)
                nx[i] = f"\\table({NEW}) {{{placement} body = {{"
                for k in range(len(body)):
                    ln = i + 1 + k
                    if k == ci:
                        nx[ln] = "} caption = " + re.match(r"\s*\\caption(.*)$", body[k]).group(1)
                    if k == ce:
                        line = nx[ln] if k == ci else body[k]
                        if li == ce: line = line[: line.rfind("@id(")]
                        nx[ln] = line.rstrip() + " trailing = {"
                    if k == li and li != ce: nx[ln] = ""
                    if k != ci and k != ce and k != li and re.fullmatch(r"\s*\\centering\s*", body[k]): nx[ln] = ""
                nx[j] = "} }"
                nt[i + 1 + li] = tl[i + 1 + li].replace(f"\\label{{{orig_id}}}", f"\\label{{{NEW}}}")
                nrefs = 0
                for xg in files:
                    tg = xg[:-5] + ".tex"
                    xs = "\n".join(nx) if xg == xf else read(xroot_dir / xg); ts = "\n".join(nt) if xg == xf else read(troot_dir / tg)
                    pat = re.compile(r"(?<=[{(,])" + re.escape(orig_id) + r"(?=[})\],])")
                    nrefs += len(re.findall(r"@ref\(" + re.escape(orig_id) + r"\)", xs))   # only checked references count; \cref stays transported
                    write(xroot_dir / xg, pat.sub(NEW, xs)); write(troot_dir / tg, pat.sub(NEW, ts))
                if nrefs < 1:
                    xs = read(xroot_dir / xf).split("\n"); ts = read(troot_dir / tf).split("\n")
                    xs[j] = xs[j] + f" See Table~@ref({NEW})."; ts[j] = ts[j] + f" See Table~\\ref{{{NEW}}}."
                    write(xroot_dir / xf, "\n".join(xs)); write(troot_dir / tf, "\n".join(ts)); nrefs = 1
                return NEW, [(xf, i + 1)], f"table float at lines {i+1}-{j+1} converted to a \\table block; original id {orig_id}; references renamed: {nrefs}"
            continue
        elif cls in ("A5-missing-figure", "A6-invalid-unit"):
            j = find_line(xl, RX_SECTION, bd + 1)
            if j is None: continue
            if cls == "A5-missing-figure":
                src = "figures/planted_ghost.pdf"; xw = "50%"; tw = "0.5\\linewidth"; name = "fig:planted_missing"
            else:
                imgs = sorted(p for p in troot_dir.rglob("*") if p.suffix.lower() in IMG_EXT and "_tectonic" not in p.parts and p.name != "main.pdf")
                if not imgs: return None, [], "no image file in the paper"
                src = str(imgs[0].relative_to(troot_dir)); xw = "3qm"; tw = "3qm"; name = "fig:planted_unit"
            xblock = f'\\figure({name}) {{ src = "{src}" width = {xw} caption = {{Planted figure}} }}'
            tblock = f"\\begin{{figure}}\\centering\\includegraphics[width={tw}]{{{src}}}\\caption{{Planted figure}}\\end{{figure}}"
            xl.insert(j + 1, xblock); tl.insert(j + 1, tblock)
            write(xroot_dir / xf, "\n".join(xl)); write(troot_dir / tf, "\n".join(tl))
            note = "" if re.search(r"\\usepackage(\[[^\]]*\])?\{[^}]*graphic[sx]", read(troot_dir / troot)) else "graphicx not loaded explicitly in the root"
            return name, [(xf, j + 2)], note + f"|insert:{xf}:{j + 2}"
    return None, [], "no plant site"

def process(pid, run_tectonic):
    e2 = json.load(open(E2 / "out" / f"{pid}.json"))
    if e2.get("failed") or e2.get("check_exit") != 0: return [dict(paper=pid, cls="*", skip="E2 twin unavailable or not clean")]
    xroot = e2["main"][:-4] + ".xtex"; troot = e2["main"]
    converted = [f["file"][:-4] + ".xtex" for f in e2["files"]]
    bib_complete = (e2.get("bibliography") or {}).get("state") == "complete"
    pw = WORK / pid; po = OUT / pid
    if pw.exists(): shutil.rmtree(pw)
    pw.mkdir(parents=True); po.mkdir(parents=True, exist_ok=True)
    # baselines
    xb = pw / "base-xtex"; shutil.copytree(E2 / "work" / pid, xb, ignore=shutil.ignore_patterns("build"))
    tb = pw / "base-tex"; shutil.copytree(CORPUS / "papers" / pid, tb)
    xcode0, xset0, _ = xtex_diags(xb, xroot)
    tlset0 = set() if XTEX_ONLY else texlab_diags(tb, troot)
    # The two linters' baseline on the clean twin: only what a plant ADDS counts.
    lint0 = {} if XTEX_ONLY and not LINTERS else {t: linter_findings(tb, troot, t, po / f"baseline.{t}.txt") for t in ("chktex", "chklref")}
    # chklref is measurable where pdflatex built the clean paper AND chklref itself finished: on
    # some papers it repeats one finding without end and dies on the 256 MB cap (exit -25) mid-report.
    chk_head = (po / "baseline.chklref.txt").read_text()[:200] if lint0.get("chklref") is not None else ""
    chk_pdf = chk_head.startswith("pdf=1") and "\nexit=0\n" in chk_head if chk_head else ""
    tec0 = tectonic_run(tb, troot) if run_tectonic else ("skipped", "", "", set(), 0)
    tectonic_ok = run_tectonic and tec0[0] == 0
    pdf0 = list((tb / "_tectonic").glob("*.pdf")) if run_tectonic else []
    write(po / "baseline.txt", f"tectonic pdf bytes={pdf0[0].stat().st_size if pdf0 else 0}\nxtex exit={xcode0} errors={sorted(xset0)}\ntexlab={sorted(tlset0)}\ntectonic exit={tec0[0]} warnings={sorted(tec0[3])} secs={tec0[4]:.0f}\n{tec0[1][-2000:]}")
    rows = []
    for cls in CLASSES_RUN:
        check_room(f"{pid}/{cls}")
        xd = pw / cls / "xtex"; td = pw / cls / "tex"
        shutil.copytree(xb, xd); shutil.copytree(tb, td, ignore=shutil.ignore_patterns("_tectonic"))
        name, sites, note = plant(cls, xd, xroot, td, troot, converted)
        ins = re.search(r"\|insert:([^:]+):(\d+)", note or "")
        note = re.sub(r"\|insert:[^:]+:\d+", "", note or "")
        row = dict(paper=pid, cls=cls, planted=name or "", sites=";".join(f"{f}:{l}" for f, l in sites), note=note, skip="" if name else note, bib_complete=bib_complete)
        if not name: rows.append(row); continue
        def shifted(base, key_file, key_line):
            if not ins: return base
            f0, l0 = ins.group(1), int(ins.group(2)); out = set()
            for d in base:
                d = list(d)
                if d[key_file] == f0 and d[key_line] >= l0: d[key_line] += 1
                out.add(tuple(d))
            return out
        xset0s = shifted(xset0, 2, 3)
        tlset0s = tlset0
        if ins:
            tf0, l0 = ins.group(1)[:-5] + ".tex", int(ins.group(2))
            tlset0s = {(a, b, c + 1 if (b == tf0 and c >= l0) else c, dd) for a, b, c, dd in tlset0}
        site_lines = {(f, l) for f, l in sites}; site_tex = {(f[:-5] + ".tex", l) for f, l in sites}
        # xtex
        xcode, xset, xerr = xtex_diags(xd, xroot); new = xset - xset0s
        hit = [d for d in new if d[1] == name or (d[2], d[3]) in site_lines]
        row.update(xtex_exit=xcode, xtex_new=len(new), xtex_detected=bool(hit), xtex_codes=";".join(sorted({d[0] for d in hit})), xtex_stderr=xerr)
        for tool in ("chktex", "chklref"):
            base = lint0.get(tool)
            if base is None:
                row[f"{tool}_new"] = ""; row[f"{tool}_any"] = ""; row[f"{tool}_msgs"] = "not installed"
                continue
            found = linter_findings(td, troot, tool, po / f"{cls}.{tool}.txt") or set()
            # Both linters print the line number inside the message; a planted insert shifts every
            # later line, so the comparison with the baseline ignores the number (as `shifted` does above).
            norm = lambda l: re.sub(r"line\s+\d+", "line N", l)
            basekeys = {norm(l) for l in base}
            fresh = [l for l in found if norm(l) not in basekeys]
            row[f"{tool}_exit"] = re.search(r"exit=(\S+)", (po / f"{cls}.{tool}.txt").read_text()[:200]).group(1)
            hit = [l for l in fresh if name in l or any(f"{f[:-5]}.tex:{l2}" in l or f":{l2}:" in l for f, l2 in sites)]
            row[f"{tool}_new"] = len(fresh); row[f"{tool}_any"] = bool(hit); row[f"{tool}_msgs"] = " | ".join(hit[:3])[:300]
        row["chklref_pdf"] = chk_pdf
        # texlab
        if XTEX_ONLY:
            base = BASE.get((pid, cls), {})
            row.update({k: base.get(k, "") for k in ("texlab_new", "texlab_any", "texlab_gating", "texlab_msgs", "tectonic_exit", "tectonic_gating", "tectonic_any", "tectonic_secs", "tectonic_evidence", "tectonic_pdf_bytes")})
            write(po / f"{cls}.diags.txt", f"planted={name} sites={sites}\nxtex new={sorted(new)}\n(texlab/tectonic columns copied from the d0e0caa pass)\n")
            rows.append(row); print(pid, cls, "xtex", row.get("xtex_detected"), row.get("xtex_codes"), flush=True)
            shutil.rmtree(xd); shutil.rmtree(td); continue
        tl = texlab_diags(td, troot); tnew = tl - tlset0s
        thit = [d for d in tnew if (d[1], d[2]) in site_tex or name in d[3]]
        row.update(texlab_new=len(tnew), texlab_any=bool(thit), texlab_gating=any(d[0] == "ERROR" for d in thit), texlab_msgs=";".join(sorted({f"{d[0]}:{d[3]}" for d in thit})))
        # tectonic
        if tectonic_ok:
            code, out, log, names, secs = tectonic_run(td, troot)
            hard = code != 0
            evidence = [l for l in (out + "\n" + log).splitlines() if name in l or "Illegal unit" in l or "planted_ghost" in l or ("multiply defined" in l and name in l)]
            pdfs = list((td / "_tectonic").glob("*.pdf")); row["tectonic_pdf_bytes"] = pdfs[0].stat().st_size if pdfs else 0
            row.update(tectonic_exit=code, tectonic_gating=hard, tectonic_any=hard or (name in names) or bool(evidence), tectonic_secs=f"{secs:.0f}", tectonic_evidence=" | ".join(evidence[:3])[:300])
            write(po / f"{cls}.tectonic.txt", out[-3000:])
        else:
            row.update(tectonic_exit="n/a", tectonic_gating="", tectonic_any="", tectonic_secs="", tectonic_evidence="baseline does not build" if run_tectonic else "skipped")
        write(po / f"{cls}.diags.txt", f"planted={name} sites={sites}\nxtex new={sorted(new)}\ntexlab new={sorted(tnew)}\n")
        rows.append(row)
        print(pid, cls, "xtex", row.get("xtex_detected"), row.get("xtex_codes"), "texlab", row.get("texlab_gating"), "tectonic", row.get("tectonic_gating"), row.get("tectonic_any"), flush=True)
        if not os.environ.get("KEEP"):
            shutil.rmtree(xd, ignore_errors=True); shutil.rmtree(td, ignore_errors=True)   # logs are in out/
    if not os.environ.get("KEEP"): shutil.rmtree(pw, ignore_errors=True)
    return rows

BASE = {}
def main():
    global BASE
    if (HERE / "out" / "results.csv").exists():
        BASE = {(r["paper"], r["cls"]): r for r in csv.DictReader(open(HERE / "out" / "results.csv"))}
    args = sys.argv[1:]; run_tectonic = "--no-tectonic" not in args and "--xtex-only" not in args
    global XTEX_ONLY, LINTERS; LINTERS = "--linters" in args; XTEX_ONLY = "--xtex-only" in args or LINTERS
    only = [a.split("=", 1)[1].split(",") for a in args if a.startswith("--classes=")]
    global CLASSES_RUN; CLASSES_RUN = only[0] if only else CLASSES
    ids = [a for a in args if not a.startswith("--")]
    OUT.mkdir(exist_ok=True); WORK.mkdir(exist_ok=True)
    papers = [r["id"] for r in csv.DictReader(open(CORPUS / "manifest.csv"))]
    ids = ids or papers
    allrows = []
    with ThreadPoolExecutor(max_workers=int(os.environ.get("WORKERS", "3"))) as ex:
        for rows in ex.map(lambda p: process(p, run_tectonic), ids): allrows.extend(rows)
    if (ids != papers or CLASSES_RUN != CLASSES) and (OUT / "results.csv").exists():   # subset run: merge into the existing table
        done = {(r["paper"], r["cls"]) for r in allrows}
        allrows = [r for r in csv.DictReader(open(OUT / "results.csv")) if (r["paper"], r["cls"]) not in done] + allrows
        order = {p: i for i, p in enumerate(papers)}; allrows.sort(key=lambda r: (order.get(r["paper"], 999), CLASSES.index(r["cls"]) if r["cls"] in CLASSES else -1))
    keys = []
    for r in allrows:
        for k in r:
            if k not in keys: keys.append(k)
    with open(OUT / "results.csv", "w", newline="") as f:
        w = csv.DictWriter(f, fieldnames=keys); w.writeheader(); w.writerows(allrows)
    print("rows:", len(allrows))

if __name__ == "__main__":
    main()
