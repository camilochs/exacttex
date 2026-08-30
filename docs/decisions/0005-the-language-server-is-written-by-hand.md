# 0005 · The language server is written by hand

**Status:** accepted, 2026-08-29. Decided by the maintainer.
**Issue:** [#16](https://github.com/camilochs/exacttex/issues/16)

---

## The decision

`xtex-lsp` implements JSON-RPC, the LSP lifecycle and every message it answers **without a dependency**.
`tower-lsp` and `lsp-types` were both available and both permissively licensed; neither is used.

The reason is control over the long run rather than cost today. A language server is where an editor meets
the compiler, and the shape of that meeting is something this project will keep changing — the diagnostics
are ours, the entity classes are ours, and the hover text is a rendering of a record `checking.md` §10
already defines. A dependency that models the protocol also models an opinion about that meeting.

The maintainer set one condition with it: **it has to stay simple enough for an agent to maintain.** That is
not a wish, it is the design constraint, and the four rules below exist to satisfy it.

## What keeps it maintainable

**1. Transcribe only what is answered.** There is no attempt to model the protocol. A message the server does
not answer has no type, no constant and no branch. Adding one is adding a case, not extending a model.

**2. One table maps a method to a handler.** A reader looking for "what happens on hover" finds one line
naming a function. There is no trait to implement, no builder, no registration order that matters.

**3. The protocol layer holds no logic.** It frames bytes, reads a method name, calls a function, writes
bytes back. Everything an editor asks about — what is at this position, what may complete here, what does
this name refer to — is answered by `xtex-core`, in functions that take bytes and offsets and return data.

That split is what makes the phase's exit criterion provable rather than argued: *the LSP and the CLI report
the same diagnostics for the same input* is true by construction when both call the same function, and only
testable by comparison when they do not.

**4. Every handled message has a test that feeds bytes and asserts bytes.** An agent adding a method copies
one, and a protocol mistake fails as a wrong byte rather than as an editor that quietly does nothing.

## What this is not

It is not a general LSP implementation and must not grow into one. If a message is not in
`docs/lsp.md`'s table, this server does not answer it, and the editor's own fallback is the correct
behaviour.

## What would reverse it

An editor feature that needs a part of the protocol large enough that transcribing it stops being cheaper
than depending on it — a full workspace-edit refactoring protocol, or streaming partial results. Reversal
means adding `lsp-types` to `xtex-lsp` alone; `xtex-core` keeps its zero dependencies either way, because
nothing in the split above depends on this choice.
