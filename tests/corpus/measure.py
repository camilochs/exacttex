#!/usr/bin/env python3
"""Measures a LaTeX corpus, and verifies a manifest against it.

Two jobs, deliberately in one file so the numbers and the fingerprints they were
computed from cannot drift apart.

    python3 measure.py <root>                 measure, and write a manifest
    python3 measure.py --verify <manifest>    check the bytes still match

The measurement asks one question per file: at what byte position does the
parser stop recognising anything? `tests/corpus/README.md` records the
thresholds, and it records them from before any corpus was measured.
"""

import argparse
import hashlib
import json
import os
import subprocess
import sys


def tex_files(root):
    for base, dirs, names in os.walk(root):
        # A corpus of documents, not of package sources. A build cache holds
        # the whole of TeX Live — pgf, expl3, tikz — and those are macro
        # implementations full of `\catcode` and `\makeatletter`, where
        # quarantining is the correct behaviour rather than a failure. Counting
        # them measures the distribution instead of the author's writing.
        dirs[:] = [
            d
            for d in dirs
            if d not in {".git", "node_modules", "target", "build"}
            and not d.startswith(".tectonic-cache")
            and d not in {"texmf", "texmf-dist"}
        ]
        for name in names:
            if name.endswith((".tex", ".xtex")):
                yield os.path.join(base, name)


def fingerprint(path):
    with open(path, "rb") as handle:
        return hashlib.sha256(handle.read()).hexdigest()


def quarantine_position(binary, path):
    """Fraction of the file available before `OpaqueToEof`, or 1.0 if never."""
    result = subprocess.run(
        [binary, "confidence", path], capture_output=True, text=True, check=False
    )
    if result.returncode != 0:
        return None
    for line in result.stdout.splitlines():
        if line.startswith("quarantine:"):
            value = line.split(":", 1)[1].strip()
            return 1.0 if value == "none" else float(value)
    return None


def measure(root, binary):
    rows = []
    for path in sorted(tex_files(root)):
        size = os.path.getsize(path)
        if size == 0:
            continue
        position = quarantine_position(binary, path)
        if position is None:
            continue
        rows.append(
            {
                "path": os.path.relpath(path, root),
                "sha256": fingerprint(path),
                "bytes": size,
                "available": position,
            }
        )
    return rows


def report(rows):
    if not rows:
        print("no files measured")
        return 1
    available = sorted(row["available"] for row in rows)
    median = available[len(available) // 2]
    early = sum(1 for value in available if value < 0.5) / len(available)

    print(f"{len(rows)} files")
    print(f"  median available before quarantine   {median:.3f}   (threshold >= 0.900)")
    print(f"  quarantined before half their bytes  {early:.1%}     (threshold <= 10%)")
    print(f"  never quarantined                    {sum(1 for v in available if v >= 1.0)}")

    failed = median < 0.90 or early > 0.10
    print("\nthresholds: " + ("MISSED" if failed else "met"))
    if failed:
        print("  Which files, and why, before changing the numbers. The thresholds")
        print("  were fixed before any corpus was measured; moving them now would")
        print("  make them a description of the result rather than a test of it.")
        for row in sorted(rows, key=lambda r: r["available"])[:10]:
            print(f"    {row['available']:.3f}  {row['path']}")
    return 1 if failed else 0


def verify(manifest_path):
    with open(manifest_path) as handle:
        manifest = json.load(handle)
    root = manifest["root"]
    changed = []
    missing = []
    for row in manifest["files"]:
        path = os.path.join(root, row["path"])
        if not os.path.exists(path):
            missing.append(row["path"])
        elif fingerprint(path) != row["sha256"]:
            changed.append(row["path"])

    print(f"{len(manifest['files'])} files in the manifest")
    print(f"  missing  {len(missing)}")
    print(f"  changed  {len(changed)}")
    for path in (missing + changed)[:10]:
        print(f"    {path}")
    if missing or changed:
        print("\nThe recorded measurement was taken from bytes that are no longer")
        print("there. Re-measure before reusing any number from it.")
        return 1
    return 0


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("target")
    parser.add_argument("--verify", action="store_true")
    parser.add_argument("--binary", default="target/debug/xtex")
    parser.add_argument("--out", default=None)
    args = parser.parse_args()

    if args.verify:
        return verify(args.target)

    rows = measure(args.target, args.binary)
    if args.out:
        with open(args.out, "w") as handle:
            json.dump({"root": os.path.abspath(args.target), "files": rows}, handle, indent=2)
        print(f"manifest written to {args.out}\n")
    return report(rows)


if __name__ == "__main__":
    sys.exit(main())
