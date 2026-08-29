#!/usr/bin/env python3
"""How often a package's commands are used without an explicit \\usepackage.

Decides whether a `needs` field could be checked against the preamble. It cannot:
a journal class or another package loads the package, and the compiler reads
neither `.cls` nor `.sty`.

    python3 measure.py <root>
"""
import collections
import os
import re
import sys

# A command distinctive enough that its presence implies the package.
PROBES = {
    "booktabs": r"\\toprule|\\midrule|\\bottomrule",
    "graphicx": r"\\includegraphics",
    "amsmath": r"\\text\{|\\begin\{align\}",
    "multirow": r"\\multirow",
    "hyperref": r"\\href|\\url\{",
    "xcolor": r"\\cellcolor|\\definecolor",
    "subcaption": r"\\subfigure|\\begin\{subfigure\}",
}


def projects(root):
    """.tex files grouped by the directory holding them."""
    found = collections.defaultdict(list)
    for base, _, names in os.walk(root):
        for name in names:
            if name.endswith(".tex"):
                found[base].append(os.path.join(base, name))
    return found


def main(root):
    grouped = projects(root)
    print(f"{'package':12}{'projects using it':>20}{'no explicit usepackage':>26}")
    for package, probe in PROBES.items():
        loads = re.compile(
            r"\\(?:usepackage|RequirePackage)(?:\[[^\]]*\])?\{[^}]*\b" + package + r"\b[^}]*\}"
        )
        used = missing = 0
        for paths in grouped.values():
            text = ""
            for path in paths:
                with open(path, errors="replace") as handle:
                    text += handle.read()
            if not re.search(probe, text):
                continue
            used += 1
            missing += 0 if loads.search(text) else 1
        share = f"({100 * missing / used:.0f}%)" if used else ""
        print(f"{package:12}{used:>20}{missing:>20} {share}")


if __name__ == "__main__":
    main(sys.argv[1] if len(sys.argv) > 1 else ".")
