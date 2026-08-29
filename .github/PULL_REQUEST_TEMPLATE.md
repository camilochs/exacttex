Fixes #
<!-- Required. Replace with the issue number (e.g. `Fixes #42`). `Closes #N`
     and `Resolves #N` work too. -->

## What
<!-- One or two sentences. The user-visible change. -->

## Why
<!-- The problem this fixes or the value this adds. If the issue already says
     it well, one line and a pointer is fine. -->

## Test plan
<!-- What you ACTUALLY ran, with the exact commands and what you saw. "Tested
     locally" is not a test plan and is grounds to send this back unreviewed.
     Paste real output where it is short enough.

     Every number reported anywhere in this PR needs the command that
     reproduces it. -->
- [ ] 

## Invariants
<!-- Tick only what you verified, and say how. An unticked box is fine; a
     ticked box you did not check is not. See AGENTS.md §4. -->
- [ ] Untouched LaTeX still comes out byte-identical (corpus transports clean, no file needed a special case)
- [ ] No annotation changes the rendered PDF or the build status
- [ ] Nothing is injected into the emitted output — no assertions, wrappers or support packages
- [ ] No hard error can originate outside an explicit ExactTeX construct
- [ ] Every diagnostic this PR adds names its blame side

## Decorrelated review
<!-- The repo has one reviewer, so the second pair of eyes is manufactured.
     Run the diff past a second model lineage before requesting review, and
     record here what it found. "Nothing found" is a valid answer. If you
     dismissed something it raised, say why. -->

## Dependencies
<!-- Any NEW third-party crate? List each with its SPDX licence, read from
     package metadata rather than recalled:

       biblatex — MIT (permissive, allowed)

     Permissive only: MIT, Apache-2.0, BSD, ISC. GPL / AGPL / SSPL / no-licence
     are never acceptable — this project is MIT. Write "None" if this PR adds
     none. -->
None

## Docs
<!-- The issue named the documentation this change had to update. List what you
     changed here. If the issue said `None`, confirm it is still true now that
     the work is done. An unmet documentation obligation blocks the merge. -->
- [ ] 

## Notes for the reviewer (optional)
<!-- Anything to focus on, or a risk you want flagged. If you made a trade-off
     deliberately, name it and say what you gave up. -->

<!-- ────────────────────────────────────────────────────────────────
     Before you request review:

       [ ] You read your own diff top to bottom.
       [ ] CI is green.
       [ ] Scope is one issue. No drive-by refactor, no drive-by reformat.
       [ ] No secrets in the diff.
       [ ] Formatter and linter pass. No bare TODOs — TODO(#N) or nothing.
       [ ] No AI attribution anywhere — no Co-Authored-By naming an agent, no
           "Generated with", no robot emoji. You are the author and you have
           read every line.

     ──────────────────────────────────────────────────────────────── -->
