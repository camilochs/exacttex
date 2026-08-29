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
import collections
import hashlib
import json
import os
import shutil
import subprocess
import sys
import tempfile


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


def confidence(binary, path):
    """Where recognition stopped, what was there, and how much is annotated."""
    result = subprocess.run(
        [binary, "confidence", path], capture_output=True, text=True, check=False
    )
    if result.returncode != 0:
        return None, None, 0.0
    position, cause, coverage = None, None, 0.0
    for line in result.stdout.splitlines():
        if line.startswith("quarantine:"):
            value = line.split(":", 1)[1].strip()
            position = 1.0 if value == "none" else float(value)
        elif line.startswith("cause:"):
            cause = line.split(":", 1)[1].strip()
        elif line.startswith("coverage:"):
            coverage = float(line.split(":", 1)[1].strip())
    return position, cause, coverage


def checks_clean(binary, path):
    """Whether unmodified LaTeX checks clean, which is the central promise.

    Run where the file actually lives, because a document's imports, includes
    and bibliography resolve relative to it. Checking a copy in a scratch
    directory would measure a document that has been taken apart.
    """
    result = subprocess.run(
        [binary, "check", os.path.basename(path)],
        cwd=os.path.dirname(path) or ".",
        capture_output=True,
        text=True,
        check=False,
    )
    return result.returncode == 0, result.stdout.strip().splitlines()[:1]


def transports(binary, path):
    """Whether emitting returns the input's bytes — property A.

    The file is copied alone into a scratch directory under a `.xtex` name,
    because emission is what the compiler writes for *this* file's bytes.
    """
    with tempfile.TemporaryDirectory() as directory:
        name = os.path.splitext(os.path.basename(path))[0] + ".xtex"
        copied = os.path.join(directory, name)
        shutil.copyfile(path, copied)
        result = subprocess.run(
            [binary, name], cwd=directory, capture_output=True, text=True, check=False
        )
        emitted = os.path.join(
            directory, "build", os.path.splitext(name)[0] + ".tex"
        )
        if result.returncode != 0 or not os.path.exists(emitted):
            return False
        with open(path, "rb") as left, open(emitted, "rb") as right:
            return left.read() == right.read()


def instrument(binary):
    """What took the measurement.

    A result file that does not say which code produced it can be picked up
    later and read as current. This has already gone wrong here three times in
    one day, each time by measuring with a binary built from a different branch
    than the one being reasoned about, so the binary's own fingerprint and the
    commit it was built from travel with every number.
    """
    with open(binary, "rb") as handle:
        digest = hashlib.sha256(handle.read()).hexdigest()
    commit = subprocess.run(
        ["git", "rev-parse", "HEAD"],
        cwd=os.path.dirname(os.path.abspath(binary)),
        capture_output=True,
        text=True,
        check=False,
    )
    dirty = subprocess.run(
        ["git", "status", "--porcelain"],
        cwd=os.path.dirname(os.path.abspath(binary)),
        capture_output=True,
        text=True,
        check=False,
    )
    return {
        "binary": os.path.abspath(binary),
        "sha256": digest,
        "commit": commit.stdout.strip() or None,
        "working_tree_clean": dirty.returncode == 0 and not dirty.stdout.strip(),
    }


def measure(root, binary):
    rows = []
    for path in sorted(tex_files(root)):
        size = os.path.getsize(path)
        if size == 0:
            continue
        position, cause, coverage = confidence(binary, path)
        if position is None:
            continue
        row = {
            "path": os.path.relpath(path, root),
            "sha256": fingerprint(path),
            "bytes": size,
            "available": position,
            "cause": cause,
            # Both promises are about LaTeX the author did not rewrite. An
            # annotated file is a different object: emitting it is *supposed*
            # to change bytes, and a hard error in it is the compiler working.
            # The compiler's own coverage figure decides which this is, rather
            # than a regular expression guessing at it here.
            "annotated": coverage > 0.0,
        }
        if not row["annotated"]:
            clean, first_line = checks_clean(binary, path)
            row["checks_clean"] = clean
            row["diagnostic"] = None if clean else (first_line[0] if first_line else "")
            row["transports"] = transports(binary, path)
        rows.append(row)
    return rows


def report(rows):
    if not rows:
        print("no files measured")
        return 1
    available = sorted(row["available"] for row in rows)
    median = available[len(available) // 2]
    early = sum(1 for value in available if value < 0.5) / len(available)

    plain = [row for row in rows if not row["annotated"]]
    dirty = [row for row in plain if not row["checks_clean"]]
    lost = [row for row in plain if not row["transports"]]

    print(f"{len(rows)} files, {len(rows) - len(plain)} of them already annotated")
    print("\n  the two promises, over the " + f"{len(plain)} unannotated files")
    print(f"    check clean, unmodified              {len(plain) - len(dirty)}/{len(plain)}")
    print(f"    emit the input's bytes exactly       {len(plain) - len(lost)}/{len(plain)}")
    print("\n  how much is reachable")
    print(f"    median available before quarantine   {median:.3f}   (threshold >= 0.900)")
    print(f"    quarantined before half their bytes  {early:.1%}     (threshold <= 10%)")
    print(f"    never quarantined                    {sum(1 for v in available if v >= 1.0)}")

    causes = collections.Counter(
        row.get("cause") for row in rows if row["available"] < 1.0 and row.get("cause")
    )
    if causes:
        print("\n  what stopped recognition, by count")
        for cause, count in causes.most_common(12):
            print(f"    {count:5}  {cause}")

    # A file that fails a promise is a defect, whatever the quarantine figure
    # says. Quarantine is a coverage signal; these two are the contract.
    failed = median < 0.90 or early > 0.10 or dirty or lost
    print("\nthresholds: " + ("MISSED" if failed else "met"))

    if dirty:
        print(f"\n  {len(dirty)} files did not check clean. Unmodified LaTeX must.")
        for row in dirty[:10]:
            print(f"    {row['path']}")
            print(f"      {row.get('diagnostic', '')}")
    if lost:
        print(f"\n  {len(lost)} files did not emit their own bytes. Transport must.")
        for row in lost[:10]:
            print(f"    {row['path']}")
    if median < 0.90 or early > 0.10:
        print("\n  Which files, and why, before changing the numbers. The thresholds")
        print("  were fixed before any corpus was measured; moving them now would")
        print("  make them a description of the result rather than a test of it.")
        for row in sorted(rows, key=lambda r: r["available"])[:10]:
            print(f"    {row['available']:.3f}  {row.get('cause', '')}  {row['path']}")
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

    # Resolved before use: two of the three measurements run with the working
    # directory set to the document's own, so that its imports and bibliography
    # resolve the way they do for its author. A relative binary path would stop
    # existing there.
    binary = os.path.abspath(args.binary)
    if not os.path.exists(binary):
        print(f"no binary at {binary}")
        return 2
    taken_by = instrument(binary)
    print(f"measured by {taken_by['commit'] or 'unknown commit'}", end="")
    print("" if taken_by["working_tree_clean"] else " (working tree dirty)")
    rows = measure(args.target, binary)
    if args.out:
        with open(args.out, "w") as handle:
            json.dump(
                {
                    "root": os.path.abspath(args.target),
                    "measured_by": instrument(binary),
                    "files": rows,
                },
                handle,
                indent=2,
            )
        print(f"manifest written to {args.out}\n")
    return report(rows)


if __name__ == "__main__":
    sys.exit(main())
