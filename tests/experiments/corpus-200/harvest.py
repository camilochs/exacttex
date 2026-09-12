#!/usr/bin/env python3
"""Corpus harvester, second campaign: 200 arXiv papers with LaTeX source under
CC BY / CC BY-SA, stratified.

The first corpus (fifty papers, `../corpus/`) was four computer-science
categories from 2022 on, and its document classes came out as it was sampled:
elsarticle, article and acmart carry 72 of them, with one llncs, three IEEEtran
and no revtex, amsart or aastex at all. Every claim about specificity is only
as wide as the templates it crossed, so this campaign samples on three axes at
once — field, year band, and the template the paper is written in — and says
where it fell short instead of pretending the quotas were met.

What is inherited: the fifty papers of the first corpus are part of these two
hundred, unchanged, so every number can be compared with the one it replaces.

Candidates come from arXiv's own bulk metadata (OAI-PMH), which carries the
licence: one request describes a few hundred papers, so the licence gate costs
nothing and only accepted papers are ever downloaded. The first campaign asked
the abs page of every candidate, one request each, and spent its time being
refused.

Politeness: >= 3 s between requests, identifying User-Agent, and a 503 with
Retry-After is honoured.
"""
import csv, gzip, io, os, re, sys, tarfile, time, urllib.error, urllib.request, xml.etree.ElementTree as ET
from collections import Counter
from pathlib import Path

HERE = Path(__file__).resolve().parent
PAPERS = HERE / "papers"; RAW = HERE / "raw"
PAPERS.mkdir(exist_ok=True); RAW.mkdir(exist_ok=True)
UA = "corrigendum-lab research (corrigendum.lab@gmail.com)"
TARGET = int(os.environ.get("TARGET", "200"))

# The strata. A quota is a target, not a promise: the licence gate decides how
# many of each field there can be, and fields differ wildly in how often authors
# choose a Creative Commons licence.
FIELDS = {
    "cs":      (["cs.CL", "cs.PL", "cs.SE", "cs.LG", "cs.DS"], 80),
    "math":    (["math.CO", "math.NA", "math.PR", "math.AP"],  30),
    "physics": (["astro-ph.GA", "cond-mat.stat-mech", "hep-th", "physics.comp-ph"], 30),
    "stat":    (["stat.ME", "stat.AP", "stat.CO"],             20),
    "bio":     (["q-bio.PE", "q-bio.QM", "q-bio.NC"],          20),
    "econ":    (["econ.EM", "econ.GN", "q-fin.ST"],            20),
}
# Two bands, because idioms age: the first corpus starts in 2022.
YEARS = [int(y) for y in os.environ.get("YEARS", "2014,2016,2018,2019,2020,2021,2022,2023,2024,2025,2026").split(",")]
PER_FIELD_YEAR = int(os.environ.get("PER_FIELD_YEAR", "6"))
# Templates the first corpus never met. A paper that brings one is taken even
# when its field's quota is full; that is the whole point of this campaign.
WANTED_CLASSES = {"revtex4-2", "revtex4-1", "revtex4", "amsart", "aastex631", "aastex63", "aastex701",
                  "llncs", "IEEEtran", "svjour3", "book", "report", "memoir", "amsbook", "elsarticle-harv"}
_last = [0.0]
_log = open(HERE / "harvest.log", "a", buffering=1)


def log(msg):
    line = f"{time.strftime('%H:%M:%S')} {msg}"
    print(line); _log.write(line + "\n")


def get(url, timeout=120):
    """One request, at most one every three seconds. A 503 with Retry-After is
    arXiv asking for room; it gets it, twice, and then the caller hears about it."""
    last_error = None
    for attempt in range(3):
        wait = 3.0 - (time.time() - _last[0])
        if wait > 0: time.sleep(wait)
        req = urllib.request.Request(url, headers={"User-Agent": UA})
        try:
            with urllib.request.urlopen(req, timeout=timeout) as r:
                return r.read(), r.headers.get("Content-Type", "")
        except urllib.error.HTTPError as e:
            last_error = e
            if e.code == 503 and attempt < 2:
                naptime = min(int(e.headers.get("Retry-After", "20") or 20), 120)
                log(f"arXiv asked for {naptime}s"); time.sleep(naptime); continue
            raise
        finally:
            _last[0] = time.time()
    raise last_error  # pragma: no cover — the loop either returns or raises


OAI = "https://oaipmh.arxiv.org/oai"
OAI_NS = {"o": "http://www.openarchives.org/OAI/2.0/", "a": "http://arxiv.org/OAI/arXiv/"}
# The sets arXiv publishes, one per field of ours.
OAI_SETS = {
    "cs": ["cs"],
    "math": ["math"],
    "physics": ["physics:astro-ph", "physics:cond-mat", "physics:hep-th", "physics:physics"],
    "stat": ["stat"],
    "bio": ["q-bio"],
    "econ": ["econ", "q-fin"],
}
OK_LICENCES = ("creativecommons.org/licenses/by/4.0", "creativecommons.org/licenses/by-sa/4.0",
               "creativecommons.org/publicdomain/zero/1.0")


WINDOWS = {"cs": 7, "math": 7, "physics": 7, "stat": 28, "bio": 28, "econ": 28}


def window(year, field):
    """A window per year, wider where the field is small: a week of economics
    holds few papers, and fewer still under a licence we may redistribute."""
    days = WINDOWS.get(field, 7)
    start = time.strptime(f"{year}-03-04", "%Y-%m-%d")
    end = time.localtime(time.mktime(start) + days * 86400)
    return time.strftime("%Y-%m-%d", start), time.strftime("%Y-%m-%d", end)


def candidates():
    """(id, field, category, licence, title) from the bulk metadata, already
    filtered to the licences that may be redistributed."""
    for field, sets in OAI_SETS.items():
        for year in YEARS:
            since, until = window(year, field)
            for st in sets:
                url = f"{OAI}?verb=ListRecords&metadataPrefix=arXiv&set={st}&from={since}&until={until}"
                taken = 0
                while url and taken < PER_FIELD_YEAR * 4:
                    try:
                        data, _ = get(url)
                    except Exception as e:
                        log(f"oai {st} {year} failed: {e}"); break
                    try:
                        root = ET.fromstring(data)
                    except ET.ParseError:
                        break
                    for rec in root.findall(".//o:record", OAI_NS):
                        meta = rec.find(".//a:arXiv", OAI_NS)
                        if meta is None: continue
                        lic = (meta.findtext("a:license", "", OAI_NS) or "").strip()
                        if not any(k in lic for k in OK_LICENCES): continue
                        aid = (meta.findtext("a:id", "", OAI_NS) or "").strip()
                        cats = (meta.findtext("a:categories", "", OAI_NS) or "").split()
                        if not aid: continue
                        title = re.sub(r"\s+", " ", meta.findtext("a:title", "", OAI_NS) or "").strip()[:200]
                        name = ("CC BY 4.0" if "licenses/by/4.0" in lic else
                                "CC BY-SA 4.0" if "licenses/by-sa/4.0" in lic else "CC0 1.0")
                        primary = cats[0] if cats else st
                        yield aid, field_of(primary, field), primary, name, title
                        taken += 1
                        if taken >= PER_FIELD_YEAR * 4: break
                    token = root.find(".//o:resumptionToken", OAI_NS)
                    url = (f"{OAI}?verb=ListRecords&resumptionToken={token.text}"
                           if token is not None and token.text else None)


def unpack(aid, data, ctype):
    dest = PAPERS / aid
    if data[:4] == b"%PDF": return None, "pdf-only"
    raw = gzip.decompress(data) if data[:2] == b"\x1f\x8b" else data
    dest.mkdir(exist_ok=True)
    if raw[257:262] == b"ustar" or ctype.endswith("x-tar") or (b"\0" in raw[:512] and len(raw) > 1024):
        try:
            with tarfile.open(fileobj=io.BytesIO(raw)) as tf:
                members = [m for m in tf.getmembers() if not (m.name.startswith("/") or ".." in m.name)]
                tf.extractall(dest, members=members)
            return dest, "tar"
        except tarfile.TarError:
            pass
    (dest / "main.tex").write_bytes(raw)
    return dest, "single"


def describe(dest):
    texs = sorted(p for p in dest.rglob("*.tex") if p.is_file())
    roots = []
    for p in texs:
        b = p.read_bytes()
        if re.search(rb"^[^%\n]*\\documentclass", b, re.M) and re.search(rb"^[^%\n]*\\begin\{document\}", b, re.M):
            score = len(re.findall(rb"\\(input|include)\b", b)) * 10 + len(b) / 1e5
            if p.name.lower() in ("main.tex", "paper.tex", "arxiv.tex"): score += 5
            roots.append((score, p))
    roots.sort(reverse=True)
    main = roots[0][1] if roots else None
    klass, babel = "", ""
    if main:
        head = main.read_bytes()[:20000].decode("utf-8", "replace")
        m = re.search(r"^[^%\n]*\\documentclass(?:\[[^\]]*\])?\{([^}]+)\}", head, re.M)
        klass = m.group(1).strip() if m else ""
        b = re.search(r"\\usepackage\[([^\]]*)\]\{babel\}", head) or re.search(r"\\usepackage\{(CJK|ctex|xeCJK)\}", head)
        babel = b.group(1).strip() if b else ""
    bibs = sorted(p for p in dest.rglob("*.bib") if p.is_file())
    bbls = sorted(p for p in dest.rglob("*.bbl") if p.is_file())
    files = [p for p in dest.rglob("*") if p.is_file()]
    return dict(files=len(files), tex_files=len(texs), bytes=sum(p.stat().st_size for p in files),
                tex_bytes=sum(p.stat().st_size for p in texs),
                main_file=str(main.relative_to(dest)) if main else "", n_roots=len(roots),
                documentclass=klass, language=babel,
                bib_files=len(bibs), bbl_files=len(bbls))


FIELDS_BY_CAT = {cat: field for field, (cats, _) in FIELDS.items() for cat in cats}


def field_of(primary, fallback):
    """A paper found in the maths set whose own primary category is eess.SP is
    an engineering paper cross-listed, and the manifest should say so."""
    if primary in FIELDS_BY_CAT: return FIELDS_BY_CAT[primary]
    head = primary.split(".")[0]
    return {"cs": "cs", "math": "math", "stat": "stat", "q-bio": "bio", "econ": "econ", "q-fin": "econ",
            "eess": "eess", "astro-ph": "physics", "cond-mat": "physics", "hep-th": "physics",
            "hep-ph": "physics", "hep-ex": "physics", "quant-ph": "physics", "physics": "physics",
            "nlin": "physics", "gr-qc": "physics", "nucl-th": "physics"}.get(head, fallback)
FIELD_QUOTA = {field: quota for field, (_, quota) in FIELDS.items()}
FIELD_QUOTA.setdefault("eess", 15)
# No template may take over the corpus: one journal's class filled twenty slots
# before anyone looked (`sigma`, 2026-09-11). Variety is the point of this corpus.
CLASS_CAP = int(os.environ.get("CLASS_CAP", "12"))


def main():
    # One harvester at a time. Two of them ran together once and both appended:
    # forty-two papers were written to the manifest twice (2026-09-11).
    lock = HERE / "harvest.lock"
    try:
        fd = os.open(lock, os.O_CREAT | os.O_EXCL | os.O_WRONLY)
        os.write(fd, str(os.getpid()).encode()); os.close(fd)
    except FileExistsError:
        log(f"another harvester holds {lock.name} ({lock.read_text().strip()}) — nothing to do")
        return
    try:
        harvest()
    finally:
        lock.unlink(missing_ok=True)


def harvest():
    man_path = HERE / "manifest.csv"; rej_path = HERE / "rejections.csv"
    fields = ["id", "field", "category", "title", "license", "unpack", "bytes", "tex_bytes", "files",
              "tex_files", "main_file", "n_roots", "documentclass", "language", "bib_files", "bbl_files",
              "source", "harvested"]
    seen, accepted = set(), []
    if man_path.exists():
        for r in csv.DictReader(open(man_path)): accepted.append(r); seen.add(r["id"])
    if rej_path.exists():
        for r in csv.DictReader(open(rej_path)): seen.add(r["id"])
    mf = open(man_path, "a", newline=""); mw = csv.DictWriter(mf, fieldnames=fields)
    if man_path.stat().st_size == 0: mw.writeheader(); mf.flush()
    rf = open(rej_path, "a", newline=""); rw = csv.DictWriter(rf, fieldnames=["id", "field", "category", "reason", "license"])
    if rej_path.stat().st_size == 0: rw.writeheader(); rf.flush()

    per_field = Counter(r.get("field", "") for r in accepted)
    classes = Counter(r.get("documentclass", "") for r in accepted)
    per_field_year = Counter()
    for aid, field, cat, lic, title in candidates():
        if len(accepted) >= TARGET: break
        if aid in seen: continue
        year_key = (field, aid[:2])
        # A field's quota holds unless the paper turns out to bring a template
        # the corpus lacks; that is decided after the download, below.
        full = per_field[field] >= FIELD_QUOTA[field]
        if full and len(accepted) < TARGET * 0.9:
            continue
        if per_field_year[year_key] >= PER_FIELD_YEAR * 3:
            continue
        seen.add(aid)
        try:
            data, ctype = get(f"https://arxiv.org/e-print/{aid}")
        except Exception as e:
            log(f"{aid} e-print failed: {e}"); rw.writerow(dict(id=aid, field=field, category=cat, reason="eprint-fetch-failed", license=lic)); rf.flush(); continue
        (RAW / f"{aid}.bin").write_bytes(data)
        dest, kind = unpack(aid, data, ctype)
        if dest is None:
            rw.writerow(dict(id=aid, field=field, category=cat, reason=kind, license=lic)); rf.flush(); continue
        d = describe(dest)
        if not d["main_file"]:
            rw.writerow(dict(id=aid, field=field, category=cat, reason="no-root-tex", license=lic)); rf.flush(); continue
        if d["documentclass"] and classes[d["documentclass"]] >= CLASS_CAP:
            rw.writerow(dict(id=aid, field=field, category=cat, reason="class-cap", license=lic)); rf.flush(); continue
        wanted = d["documentclass"] in WANTED_CLASSES and classes[d["documentclass"]] < 8
        if full and not wanted:
            rw.writerow(dict(id=aid, field=field, category=cat, reason="field-quota-full", license=lic)); rf.flush(); continue
        row = dict(id=aid, field=field, category=cat, title=title, license=lic, unpack=kind,
                   source="harvest-200", harvested=time.strftime("%Y-%m-%d"), **d)
        mw.writerow(row); mf.flush(); accepted.append(row)
        per_field[field] += 1; per_field_year[year_key] += 1; classes[d["documentclass"]] += 1
        log(f"{aid} [{field}/{cat}] ACCEPT {lic} class={d['documentclass'] or '?'} "
            f"tex={d['tex_files']} bib={d['bib_files']} ({len(accepted)}/{TARGET})")
    log(f"done: {len(accepted)} accepted · fields {dict(per_field)} · classes {classes.most_common(12)}")


if __name__ == "__main__":
    main()
