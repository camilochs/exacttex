#!/usr/bin/env python3
"""Builds a corpus of LaTeX written by other people, across years and fields.

Everything the compiler knows so far comes from one author. This fetches
documents nobody here wrote, stratified two ways, because LaTeX practice varies
along both axes and a corpus that varies along neither measures a style rather
than a language.

    python3 fetch.py --out <directory>
    python3 fetch.py --out <directory> --per-cell 2

Nothing is downloaded into the repository. `tests/corpus/README.md` records why:
these documents' licences are not ours to grant, so the corpus lives outside and
only its fingerprints come back. `provenance.json` is what makes a measurement
taken from it re-checkable — identifier, field, year, licence, URL, and a
SHA-256 of every file measured.

arXiv's API terms of use ask for one request every three seconds over a single
connection, and its bulk-access page lists crawling the export service as a
sanctioned route for a subset of content. This makes one request at a time and
sleeps between them. A larger corpus belongs on their S3 bulk data, not here.
"""

import argparse
import gzip
import hashlib
import io
import json
import os
import random
import re
import subprocess
import sys
import tarfile
import time
import urllib.error
import urllib.parse
import urllib.request

# Eight cuts from the pdflatex era to now. LaTeX practice moved underneath them:
# pdflatex gave way to xelatex and lualatex, natbib to biblatex, TikZ arrived,
# and source files stopped being ASCII.
YEARS = [2005, 2008, 2011, 2014, 2017, 2020, 2023, 2026]

# Six fields that write LaTeX differently: theorem environments and private
# macro layers in mathematics, algorithms and listings in computer science,
# revtex and dense display maths in physics, aastex and wide tables in
# astronomy. All six existed in 2005, so no cell of the grid is empty for a
# reason that has nothing to do with the compiler.
# `astro-ph*` rather than `astro-ph`: the archive was split into subcategories
# in 2009, and the bare name matches nothing after that — a query that silently
# returns an empty year rather than an error. Every pair below was checked at
# both ends of the range before the corpus was built, and all twelve return
# results.
AREAS = ["math.AG", "cs.DS", "hep-th", "astro-ph*", "cond-mat.stat-mech", "q-bio.PE"]

# Whole documents rather than papers: books and lecture notes, multi-file, held
# together by \include. arXiv has almost none of that shape, and it is the shape
# the label inventory crosses. Pinned by name and licence here rather than
# discovered by search, so the selection can be audited rather than trusted.
REPOSITORIES = [
    ("OpenLogicProject/OpenLogic", "CC-BY-4.0"),
    ("sysprog21/lkmpg", "OSL-3.0"),
    ("dendibakh/perf-book", "CC0-1.0"),
    ("MasatakaYm/Molecular-Simulation", "CC-BY-4.0"),
    ("yegor256/ssd16", "MIT"),
]

SEED = 20260829
DELAY = 3.0  # seconds between requests to arxiv.org, per their terms of use
AGENT = "exacttex-corpus/0.1 (research measurement; camilochs@gmail.com)"

_last_request = [0.0]


def polite_get(url, params=None):
    """One request at a time, never faster than the published rate."""
    if params:
        url = url + "?" + urllib.parse.urlencode(params)
    waited = time.monotonic() - _last_request[0]
    if waited < DELAY:
        time.sleep(DELAY - waited)
    request = urllib.request.Request(url, headers={"User-Agent": AGENT})
    try:
        with urllib.request.urlopen(request, timeout=120) as response:
            body = response.read()
    except (urllib.error.URLError, TimeoutError) as error:
        _last_request[0] = time.monotonic()
        return None, str(error)
    _last_request[0] = time.monotonic()
    return body, None


def ids_for(area, year, want):
    """Papers submitted to `area` during `year`, sampled reproducibly.

    Returns each paper's own primary category alongside its identifier. A search
    for a category matches papers cross-listed into it, so the field a paper was
    drawn from is not necessarily the field it belongs to, and recording the
    query instead of the answer would put a gr-qc paper in the astronomy row.
    """
    body, error = polite_get(
        "https://export.arxiv.org/api/query",
        {
            "search_query": f"cat:{area} AND submittedDate:"
            f"[{year}01010000 TO {year}12312359]",
            "start": 0,
            # Sampling from a pool rather than taking the first few: the API
            # returns them in a fixed order, and the first few of any year are
            # the first few days of January.
            "max_results": 60,
        },
    )
    if body is None:
        print(f"  {area} {year}: search failed, {error}")
        return []
    pool = []
    for entry in body.split(b"<entry>")[1:]:
        identifier = re.search(rb"<id>http://arxiv\.org/abs/([^<]+?)v\d+</id>", entry)
        primary = re.search(rb'primary_category[^/>]*term="([^"]*)"', entry)
        if identifier:
            pool.append(
                (
                    identifier.group(1).decode(),
                    primary.group(1).decode() if primary else "unknown",
                )
            )
    if not pool:
        print(f"  {area} {year}: no results")
        return []
    if len(pool) <= want:
        return pool
    return random.Random(f"{SEED}-{area}-{year}").sample(pool, want)


def licence_of(identifier):
    """The licence arXiv records for a paper, transcribed rather than assumed."""
    body, error = polite_get(
        "https://export.arxiv.org/oai2",
        {
            "verb": "GetRecord",
            "identifier": f"oai:arXiv.org:{identifier}",
            "metadataPrefix": "arXiv",
        },
    )
    if body is None:
        return None, None, error
    licence = re.search(rb"<license>([^<]*)</license>", body)
    created = re.search(rb"<created>([^<]*)</created>", body)
    return (
        licence.group(1).decode() if licence else "arxiv-default",
        created.group(1).decode() if created else None,
        None,
    )


def tex_from_eprint(body):
    """The `.tex` files in an e-print, whatever shape it arrived in.

    A submission is a gzipped tar, a single gzipped file, or a PDF. The last is
    a PDF-only submission with no source to measure, and is skipped rather than
    counted as a failure of anything.
    """
    if body[:4] == b"%PDF":
        return None, "pdf-only submission"
    try:
        plain = gzip.decompress(body)
    except (OSError, EOFError) as error:
        return None, f"not gzip: {error}"
    try:
        with tarfile.open(fileobj=io.BytesIO(plain)) as archive:
            files = {}
            for member in archive.getmembers():
                if not member.isfile() or not member.name.endswith(".tex"):
                    continue
                # A tar member's name is attacker-controlled in general. These
                # come from arXiv, but a path that escapes the directory is
                # still a path we refuse to write.
                if os.path.isabs(member.name) or ".." in member.name.split("/"):
                    continue
                handle = archive.extractfile(member)
                if handle is not None:
                    files[member.name] = handle.read()
        if not files:
            return None, "no .tex in the archive"
        return files, None
    except tarfile.TarError:
        # Not a tar: a single-file submission, gzipped on its own.
        return {"main.tex": plain}, None


def fingerprint(data):
    return hashlib.sha256(data).hexdigest()


def fetch_arxiv(out, per_cell):
    documents = []
    skipped = []
    for year in YEARS:
        for area in AREAS:
            print(f"{year} {area}")
            for identifier, primary in ids_for(area, year, per_cell):
                directory = os.path.join(out, "arxiv", str(year), area, identifier.replace("/", "_"))
                if os.path.isdir(directory) and os.listdir(directory):
                    print(f"  {identifier}: already here")
                    continue
                body, error = polite_get(f"https://arxiv.org/e-print/{identifier}")
                if body is None:
                    skipped.append((identifier, f"download failed: {error}"))
                    print(f"  {identifier}: {error}")
                    continue
                files, reason = tex_from_eprint(body)
                if files is None:
                    skipped.append((identifier, reason))
                    print(f"  {identifier}: {reason}")
                    continue
                licence, created, error = licence_of(identifier)
                if error:
                    skipped.append((identifier, f"licence unknown: {error}"))
                    print(f"  {identifier}: licence unknown, not kept")
                    continue

                os.makedirs(directory, exist_ok=True)
                recorded = []
                for name, data in files.items():
                    path = os.path.join(directory, name)
                    os.makedirs(os.path.dirname(path), exist_ok=True)
                    with open(path, "wb") as handle:
                        handle.write(data)
                    recorded.append(
                        {
                            "path": os.path.relpath(path, out),
                            "bytes": len(data),
                            "sha256": fingerprint(data),
                        }
                    )
                documents.append(
                    {
                        "source": "arxiv",
                        "id": identifier,
                        "area": primary,
                        "drawn_from": area,
                        "year": year,
                        "created": created,
                        "licence": licence,
                        "url": f"https://arxiv.org/abs/{identifier}",
                        "files": recorded,
                    }
                )
                print(f"  {identifier} [{primary}]: {len(recorded)} .tex, {licence}")
    return documents, skipped


def fetch_repositories(out):
    documents = []
    skipped = []
    for name, licence in REPOSITORIES:
        directory = os.path.join(out, "github", name.replace("/", "_"))
        if not os.path.isdir(directory):
            print(f"cloning {name}")
            result = subprocess.run(
                ["git", "clone", "--depth", "1", f"https://github.com/{name}.git", directory],
                capture_output=True,
                text=True,
                check=False,
            )
            if result.returncode != 0:
                skipped.append((name, result.stderr.strip().splitlines()[-1:]))
                print(f"  {name}: clone failed")
                continue
        commit = subprocess.run(
            ["git", "-C", directory, "rev-parse", "HEAD"],
            capture_output=True,
            text=True,
            check=False,
        ).stdout.strip()

        recorded = []
        for base, dirs, names in os.walk(directory):
            dirs[:] = [d for d in dirs if d != ".git"]
            for name_ in names:
                if not name_.endswith(".tex"):
                    continue
                path = os.path.join(base, name_)
                with open(path, "rb") as handle:
                    data = handle.read()
                recorded.append(
                    {
                        "path": os.path.relpath(path, out),
                        "bytes": len(data),
                        "sha256": fingerprint(data),
                    }
                )
        documents.append(
            {
                "source": "github",
                "id": name,
                "area": "book",
                "year": None,
                "created": None,
                "licence": licence,
                "url": f"https://github.com/{name}/tree/{commit}",
                "commit": commit,
                "files": recorded,
            }
        )
        print(f"  {name}: {len(recorded)} .tex at {commit[:8]}")
    return documents, skipped


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--out", required=True)
    parser.add_argument("--per-cell", type=int, default=2)
    parser.add_argument("--skip-arxiv", action="store_true")
    parser.add_argument("--skip-github", action="store_true")
    args = parser.parse_args()

    os.makedirs(args.out, exist_ok=True)
    documents, skipped = [], []
    if not args.skip_arxiv:
        found, missed = fetch_arxiv(args.out, args.per_cell)
        documents += found
        skipped += missed
    if not args.skip_github:
        found, missed = fetch_repositories(args.out)
        documents += found
        skipped += missed

    path = os.path.join(args.out, "provenance.json")
    with open(path, "w") as handle:
        json.dump(
            {
                "seed": SEED,
                "years": YEARS,
                "areas": AREAS,
                "documents": documents,
                "skipped": [{"id": i, "reason": str(r)} for i, r in skipped],
            },
            handle,
            indent=2,
        )

    files = sum(len(d["files"]) for d in documents)
    print(f"\n{len(documents)} documents, {files} .tex files")
    print(f"  skipped {len(skipped)}")
    print(f"  provenance written to {path}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
