#!/usr/bin/env python3
r"""Migrates a LaTeX file to ExactTeX annotations, the way an author would.

The generator for property B. Real papers are already labelled, so there is
nothing to *add* to them — what an author actually does is convert:

    \caption{Runtime}          \caption{Runtime}
    \label{fig:runtime}   ->   @id(fig:runtime)

`@id(x)` emits `\label{x}`, so the built document should be the same document
and the pages should be identical. That is property B stated as the thing
someone would really do on their first day.

Two shapes, both found in real projects:

    1. the label follows its anchor    \caption{Runtime}\label{fig:x}
    2. the label opens the caption     \caption{\label{fig:x}Runtime}

The second is attempted only when no space follows the label, and that
restriction was measured rather than guessed. `\label` prints nothing but it is
present, so a space after it inside a caption is typeset; move the label out and
that space merges with the one `\caption` already placed. Every caption's first
word shifts, and the pixel comparison caught it on nine pages of a real paper.

A caption written `\caption{\label{x} Text}` is therefore not migratable by
this transformation, and the harness leaves it alone.

`\ref` is never touched. `\ref` produces ink, and a test that changed one would
be measuring a real difference and calling it a bug.

Where the compiler will not recognise the result — an anchor separated from its
label by a comment, which `@id` does not skip — the conversion is still made
and the harness skips the case, because the compiler is the authority on
eligibility rather than this pattern.
"""

import re
import sys

NAME = r"[A-Za-z][A-Za-z0-9_:.\-]*"
ANCHOR = r"\\(?:caption|section|subsection|subsubsection|chapter)\*?"
BRACED = r"\{(?:[^{}]|\{[^{}]*\})*\}"

AFTER = re.compile(rf"({ANCHOR}{BRACED})(\s{{0,80}}?)\\label\{{({NAME})\}}")
# Only where no space follows the label. `\label` prints nothing but it is
# *present*, so the space after it is typeset; move the label out and that space
# merges with the one `\caption` already placed, and every caption's first word
# shifts. Measured on nine pages of a real paper before this rule existed.
INSIDE = re.compile(rf"\\caption\{{\\label\{{({NAME})\}}(\S(?:[^{{}}]|\{{[^{{}}]*\}})*)\}}")


def migrate(text):
    """Returns the migrated text and how many labels were converted."""
    count = 0

    def inside(match):
        nonlocal count
        count += 1
        return f"\\caption{{{match.group(2)}}} @id({match.group(1)})"

    def after(match):
        nonlocal count
        count += 1
        return f"{match.group(1)}{match.group(2)}@id({match.group(3)})"

    text = INSIDE.sub(inside, text)
    return AFTER.sub(after, text), count


def main():
    if len(sys.argv) != 3:
        print("usage: annotate.py <in.tex> <out.xtex>", file=sys.stderr)
        return 2
    with open(sys.argv[1], errors="replace") as handle:
        text = handle.read()
    migrated, count = migrate(text)
    with open(sys.argv[2], "w") as handle:
        handle.write(migrated)
    print(count)
    return 0


if __name__ == "__main__":
    sys.exit(main())
