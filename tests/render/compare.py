#!/usr/bin/env python3
"""Property B: annotating must not change a rendered pixel.

    render(tex(emit(d)))  ==  render(tex(emit(erase(d))))

Taking a `.tex` that compiles, adding annotations, and building must produce
the same pages. This takes a corpus of real documents, annotates each, builds
both versions, rasterises both, and compares.

    python3 compare.py <root> --binary <xtex> [--limit N]

Both builds go through the same engine at the same settings, and the comparison
is on rasterised pages rather than on PDF bytes, because two identical builds
already differ in metadata and an ID while rendering the same ink. That is
measured rather than assumed — see `tests/render/README.md`.
"""

import argparse
import os
import shutil
import subprocess
import sys
import tempfile

TECTONIC = "/opt/homebrew/bin/tectonic"
PDFTOPPM = "/opt/homebrew/bin/pdftoppm"
HERE = os.path.dirname(os.path.abspath(__file__))


def run(args, cwd):
    return subprocess.run(args, cwd=cwd, capture_output=True, text=True, check=False)


def compile_pdf(directory, name):
    """Compiles `name` in `directory`, returning the PDF path or None."""
    result = run([TECTONIC, "-X", "compile", name, "--outfmt", "pdf"], directory)
    pdf = os.path.join(directory, name.rsplit(".", 1)[0] + ".pdf")
    return pdf if result.returncode == 0 and os.path.exists(pdf) else None


def rasterise(pdf, prefix):
    run([PDFTOPPM, "-r", "150", "-png", "-gray", pdf, prefix], os.path.dirname(pdf))
    directory = os.path.dirname(pdf)
    base = os.path.basename(prefix)
    return sorted(
        os.path.join(directory, name)
        for name in os.listdir(directory)
        if name.startswith(base) and name.endswith(".png")
    )


def pages_differ(left, right):
    """Returns a reason the two page sets differ, or None."""
    from PIL import Image, ImageChops

    if len(left) != len(right):
        return f"page count {len(left)} vs {len(right)}"
    for index, (a, b) in enumerate(zip(left, right), start=1):
        first, second = Image.open(a), Image.open(b)
        if first.size != second.size:
            return f"page {index} size {first.size} vs {second.size}"
        if ImageChops.difference(first, second).getbbox() is not None:
            return f"page {index} differs in pixels"
    return None


def check(path, binary):
    """One document. Returns (verdict, detail)."""
    with tempfile.TemporaryDirectory() as parent:
        # The whole project, not the file. A paper needs its figures, its
        # `.bib` and often its class file, and a copy of the `.tex` alone does
        # not compile — which would skip every real document and leave the
        # suite proving nothing.
        work = os.path.join(parent, "project")
        shutil.copytree(
            os.path.dirname(os.path.abspath(path)),
            work,
            # Not `*.pdf`: a paper's figures are usually PDFs, and excluding
            # them removes the images and then reports that the document does
            # not compile. That skipped every real paper in the first sweep.
            ignore=shutil.ignore_patterns("build", ".git"),
            symlinks=True,
        )
        original = os.path.join(work, "original.tex")
        shutil.copy(path, original)

        annotated = os.path.join(work, "annotated.xtex")
        made = run(
            ["python3", os.path.join(HERE, "annotate.py"), original, annotated], work
        )
        if made.returncode != 0 or made.stdout.strip() == "0":
            return "skipped", "nothing eligible to annotate"

        # The original must build, or there is no baseline to compare against
        # and the document tells us nothing about annotating.
        base_pdf = compile_pdf(work, "original.tex")
        if base_pdf is None:
            return "skipped", "the original does not compile here"

        emitted = run([binary, "annotated.xtex"], work)
        built = os.path.join(work, "build", "annotated.tex")
        if emitted.returncode != 0 or not os.path.exists(built):
            return "failed", "emission failed"

        # The compiler is the authority on where an annotation is eligible, not
        # the pattern that inserted it. An `@id` landing somewhere recognition
        # is disabled — inside a `\newcommand` body, say — is transported
        # literally and prints, and comparing that would be measuring the
        # generator rather than the property.
        with open(built, errors="replace") as handle:
            output = handle.read()
        if "@id(" in output:
            return "skipped", "an annotation landed where recognition is disabled"

        shutil.copy(built, os.path.join(work, "emitted.tex"))

        emitted_pdf = compile_pdf(work, "emitted.tex")
        if emitted_pdf is None:
            # The property covers build status as well as pixels: annotating
            # must not turn a passing build into a failing one.
            return "failed", "the annotated build failed where the original passed"

        reason = pages_differ(
            rasterise(base_pdf, os.path.join(work, "base")),
            rasterise(emitted_pdf, os.path.join(work, "ann")),
        )
        return ("failed", reason) if reason else ("equal", made.stdout.strip())


def documents(root):
    for base, dirs, names in os.walk(root):
        dirs[:] = [
            d
            for d in dirs
            if d not in {".git", "node_modules", "target", "build"}
            and not d.startswith(".tectonic-cache")
        ]
        for name in names:
            if name.endswith(".tex"):
                yield os.path.join(base, name)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("root")
    parser.add_argument("--binary", required=True)
    parser.add_argument("--limit", type=int, default=0)
    args = parser.parse_args()

    tally = {"equal": 0, "failed": 0, "skipped": 0}
    reasons = {}
    failures = []
    for index, path in enumerate(sorted(documents(args.root))):
        if args.limit and index >= args.limit:
            break
        verdict, detail = check(path, args.binary)
        tally[verdict] += 1
        reasons[detail] = reasons.get(detail, 0) + 1 if verdict == "skipped" else reasons.get(detail, 0)
        if verdict == "failed":
            failures.append((path, detail))
            print(f"FAILED {os.path.relpath(path, args.root)}: {detail}", flush=True)

    print(f"\nequal {tally['equal']}   failed {tally['failed']}   skipped {tally['skipped']}")
    for reason, count in sorted(reasons.items(), key=lambda r: -r[1]):
        if count:
            print(f"  skipped: {reason} — {count}")
    if failures:
        print("\nAnnotating changed the output. That is property B failing, and")
        print("the annotation is the suspect rather than the document.")
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
