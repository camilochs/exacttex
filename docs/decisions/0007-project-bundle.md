# 0007 · The module is handed a project, whole, on every call

**Status:** accepted, 2026-08-30.
**Issue:** [#69](https://github.com/camilochs/exacttex/issues/69)

Two shapes were considered for getting a multi-file project into the WebAssembly module.

**Pass it in whole** — a length-prefixed list of name and bytes, on every call. Stateless, no imported
functions.

**Call back into the host per file.** The only thing this buys is not knowing the file set in advance —
and the host is an editor, which is the thing that opened the files. Measured against a real Springer
monograph: 388 KB across 20 files, checked in 71 ms *including twenty process starts*; in-process there is
nothing to save. And WebAssembly imports are synchronous, so a file arriving over a network cannot be
awaited from inside one without heavy machinery. The shape that appears to serve the network case serves
it worst.

So: the project is passed in whole. The wire format is in [`wasm.md`](../wasm.md) — little-endian,
length-prefixed, unaligned, readable with a `DataView` and a loop.

Two consequences worth naming:

1. **The pipeline moved into the core.** `check_project` — transitive imports, the author's `\include`
   edges, the bibliography, the label inventory — lived in the CLI and is now `xtex_core::project`,
   parameterised over `SourceLoader`. The CLI's loader answers from the filesystem; the module's from the
   bundle. The parity test compares the CLI binary on a project on disk against the module on the same
   project as a bundle: two hosts, one answer, byte for byte.
2. **Assets are present by name.** A figure's PDF is listed with empty data. Existence is the only
   question ever asked of an asset, and omitting it would make a correct `src` field a false hard error.
