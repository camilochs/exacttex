#!/usr/bin/env python3
"""Does the `prefix:` convention hold well enough to be a type demand?

Decides whether `@ref(fig:x)` can mean "this must be a figure". Two questions:
how many labels carry a prefix at all, and how often the prefix agrees with the
environment the label sits in.

    python3 measure.py <root>
"""
import collections
import os
import re
import sys

ENV = re.compile(
    r"\\begin\{(figure|table|algorithm|equation|align)\*?\}(.*?)\\end\{\1\*?\}", re.S
)
LABEL = re.compile(r"\\label\{([^}]*)\}")


def strip_comments(text):
    return "\n".join(re.sub(r"(?<!\\)%.*$", "", line) for line in text.split("\n"))


def tex_files(root):
    for base, _, names in os.walk(root):
        for name in names:
            if name.endswith(".tex"):
                yield os.path.join(base, name)


def main(root):
    prefixes = collections.Counter()
    inside = collections.defaultdict(collections.Counter)
    total = 0

    for path in tex_files(root):
        with open(path, errors="replace") as handle:
            text = strip_comments(handle.read())
        for match in LABEL.finditer(text):
            total += 1
            name = match.group(1)
            prefixes[name.split(":")[0].lower() if ":" in name else "(none)"] += 1
        for match in ENV.finditer(text):
            kind, body = match.group(1), match.group(2)
            for label in LABEL.finditer(body):
                name = label.group(1)
                inside[kind][name.split(":")[0].lower() if ":" in name else "(none)"] += 1

    print(f"{total} labels\n")
    for prefix, count in prefixes.most_common():
        print(f"  {prefix:16}{count:5}")

    print("\nagreement between prefix and environment:")
    for kind in sorted(inside):
        counts = inside[kind]
        subtotal = sum(counts.values())
        top, best = counts.most_common(1)[0]
        print(f"  {kind:12}{subtotal:5} labelled   {top}: {best} ({100 * best / subtotal:.0f}%)")


if __name__ == "__main__":
    main(sys.argv[1] if len(sys.argv) > 1 else ".")
