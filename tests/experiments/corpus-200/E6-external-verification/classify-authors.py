#!/usr/bin/env python3
"""Author-list differences, classified by rule rather than by reading.

The verifier flags every author-list difference at high severity, and most of
them are not errors in the paper: a registry that returns only the first
author, a consortium the registry drops, an accent written as a TeX escape.
Reading them one by one is not a measurement, so this classifies them with
rules over the two lists, after folding what formatting varies and truth does
not: TeX accents decoded, braces dropped, "Last, First" reordered, initials
reduced to the family name.

The classes are ordered from "the paper is wrong" to "the registry is thin":

  missing-author      the document's list is a subset of the source's; the
                      paper omits someone the registry lists
  different-work      the two lists share no family name at all
  name-differs        same length, one family name differs
  registry-truncated  the source's list is a subset of the document's; the
                      registry returned fewer authors, which is not the
                      paper's error
  consortium-only     the only difference is a corporate or collaboration
                      author (initiative, group, consortium, collaboration)
  formatting-only     the same family names in the same order
"""
import csv, re, sys, unicodedata
from collections import Counter
from pathlib import Path

HERE = Path(__file__).resolve().parent
CORPORATE = re.compile(r"initiative|consorti|collaborat|group|team|network|study|project|committee|society", re.I)


ACCENTS = re.compile(r"\\[\"'`^~=.uvHckrb]\s*")


def abbreviated(s: str) -> bool:
    """The document itself says the list is cut: `and others`, `et al.`"""
    return bool(re.search(r"\band\s+others\b|\bet\s+al\.?", s, re.I))


def firsts(s: str) -> list[str]:
    """The other reading of a name: a registry that prints `Xing Chao` for
    `Chao Xing` has put the family name first."""
    out = []
    for part in re.split(r"\s+and\s+|;", ACCENTS.sub("", s).replace("{", "").replace("}", "")):
        part = part.strip().strip(",.")
        if not part or CORPORATE.search(part): continue
        w = part.split(",")[1].split() if "," in part and len(part.split(",")) > 1 else part.split()
        if w: out.append(re.sub(r"[^a-zA-Z\-]", "", w[0]).lower())
    return out


def families(s: str) -> list[str]:
    s = ACCENTS.sub("", s)                                 # \"a, \'e, \v{c} …
    s = re.sub(r"\\[a-zA-Z]+\s*", "", s)                   # \textit, \relax …
    s = s.replace("{", "").replace("}", "").replace("~", " ")
    s = s.replace("\ufffd", "")                            # a registry that mangled its own encoding
    s = unicodedata.normalize("NFKD", s)
    s = "".join(ch for ch in s if not unicodedata.combining(ch))
    out = []
    for part in re.split(r"\s+and\s+|;", s):
        part = part.strip().strip(",.")
        if not part or part.lower() in ("others", "et al", "et al."):
            continue
        if CORPORATE.search(part):
            out.append("~corporate~"); continue
        if "," in part:
            fam = part.split(",")[0]
        else:
            words = part.split()
            fam = words[-1] if words else ""
        fam = re.sub(r"[^a-zA-Z\- ]", "", fam).strip().lower()
        if fam: out.append(fam)
    return out


def near(a: str, b: str) -> bool:
    """Two family names one or two keystrokes apart: a registry that dropped a
    letter (`Daru` for `Darup`) is the registry's slip, not the paper's."""
    if a == b: return True
    if abs(len(a) - len(b)) > 2: return False
    prev = list(range(len(b) + 1))
    for i, ca in enumerate(a, 1):
        cur = [i]
        for j, cb in enumerate(b, 1):
            cur.append(min(prev[j] + 1, cur[j - 1] + 1, prev[j - 1] + (ca != cb)))
        prev = cur
    return prev[-1] <= 2


def classify(doc: str, src: str) -> str:
    d, s = families(doc), families(src)
    if abbreviated(doc) and set(d) <= set(s): return "abbreviated-in-document"
    if set(d) and set(d) == set(firsts(src)): return "name-order-in-registry"
    if d == s: return "formatting-only"
    dset, sset = set(d), set(s)
    if dset == sset: return "formatting-only"
    if (dset - sset) | (sset - dset) <= {"~corporate~"}: return "consortium-only"
    dp, sp = [x for x in d if x != "~corporate~"], [x for x in s if x != "~corporate~"]
    dset, sset = set(dp), set(sp)
    if not dset or not sset: return "consortium-only"
    if dset == sset: return "formatting-only"
    if dset < sset: return "missing-author"
    if sset < dset: return "registry-truncated"
    if not (dset & sset): return "different-work"
    if len(dp) == len(sp) and all(near(a, b) for a, b in zip(dp, sp)): return "spelling-only"
    return "name-differs"


def main() -> None:
    rows = [r for r in csv.DictReader(open(HERE / "out" / "diffs-all.csv"))
            if r["field"] in ("author", "authors")]
    counts, out = Counter(), []
    for r in rows:
        cls = classify(r["in_document"], r["in_source"])
        counts[cls] += 1
        out.append({**r, "class": cls})
    with open(HERE / "out" / "author-classes.csv", "w", newline="") as f:
        w = csv.DictWriter(f, fieldnames=list(out[0].keys())); w.writeheader(); w.writerows(out)
    print(f"{len(rows)} author-list differences, by rule:")
    for k, n in counts.most_common():
        print(f"  {k:20s} {n:4d}  {100*n/len(rows):4.0f}%")
    # A row the verifier reached by searching a title is its guess about which
    # work the entry means; only a row reached through the entry's own DOI can
    # say the paper is wrong (see the compiler's XT1018 rule).
    by_source = Counter((r["class"], "by-doi" if r["source"] != "crossref-query" else "by-search") for r in out)
    print("\nby how the record was reached:")
    for (cls, how), n in sorted(by_source.items()):
        print(f"  {cls:20s} {how:10s} {n:4d}")
    candidates = [r for r in out if r["class"] in ("missing-author", "different-work", "name-differs")
                  and r["source"] != "crossref-query"]
    print(f"\ncandidates for a real error: {len(candidates)} in {len({r['paper'] for r in candidates})} papers")
    for r in candidates:
        print(f"  {r['paper']} {r['key']:28s} {r['class']:18s} doc: {r['in_document'][:60]!r} src: {r['in_source'][:60]!r}")


if __name__ == "__main__":
    main()
