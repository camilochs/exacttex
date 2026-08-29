# The WebAssembly build

The same compiler, as a `.wasm` file the browser runs. No compile server, and no
`wasm-bindgen`.

```sh
rustup target add wasm32-unknown-unknown
cargo build -p xtex-wasm --target wasm32-unknown-unknown --release
# target/wasm32-unknown-unknown/release/xtex_wasm.wasm — about 140 KB
```

There is no build step after that. No bundler, no generated glue, no `npm install`. The module is a file with
six exports and a linear memory.

---

## The calling convention

1. `xtex_alloc(len)` returns a pointer to `len` writable bytes.
2. Copy a **project bundle** there.
3. Call an operation with `(pointer, len)`. It returns a **result pointer**: four little-endian bytes of
   length, then that many bytes.
4. Read them, then `xtex_free_result(result)` and `xtex_free(pointer, len)`.

The length prefix is not decoration. Emitted LaTeX can contain a zero byte, and a C-string convention would
truncate the document at it.

## The project bundle

Everything that makes the compiler worth using is multi-file, and a browser has no filesystem, so the host
supplies the whole project on every call. Decided in [`decisions/0007`](decisions/0007-project-bundle.md):
a real 20-file monograph is 388 KB and checks in milliseconds, so there is nothing to save by reading
lazily, and WebAssembly imports are synchronous, so a per-file callback could not await a network anyway.

The format is length-prefixed, little-endian, unaligned — a `DataView` and a loop:

```text
u32 root_len    root_name (UTF-8)
u32 file_count
file_count × ( u32 name_len   name (UTF-8)   u32 data_len   data )
```

- Names are logical, `/`-separated, project-relative — the same names the project's own `@import` and
  `\include` write. A single file is a one-entry bundle, not a special case.
- The host includes **every file a check may ask about**. An asset that exists but is not source — a
  figure's PDF — is listed with empty data, because existence is the only question ever asked of it, and
  omitting it makes `src = "figures/plot.pdf"` a false hard error.
- A malformed bundle — a length past the end, trailing bytes, a non-UTF-8 name — returns the empty result.
  It is the caller's bug, not the author's document, and answering anyway would answer the wrong question.

| Export | Takes | Returns |
|---|---|---|
| `xtex_alloc(len)` | — | a pointer to writable bytes |
| `xtex_free(ptr, len)` | — | — |
| `xtex_free_result(ptr)` | — | — |
| `xtex_emit(ptr, len)` | a bundle | the root's emitted LaTeX |
| `xtex_check_json(ptr, len)` | a bundle | the JSON `xtex check --json` prints, for the whole project |
| `xtex_source_map(ptr, len)` | a bundle | the root's source map, as JSON |
| `xtex_blame(ptr, len)` | two length-prefixed texts, then a bundle | the engine's records, translated |
| `xtex_rename_plan(ptr, len)` | `from`, `to`, then a bundle | the plan, edits and untouched alike |
| `xtex_rename_apply(ptr, len)` | `from`, `to`, `target`, then a bundle | the target file's rewritten bytes |
| `xtex_hover(ptr, len)` | `target`, `u32 offset`, then a bundle | hover text |
| `xtex_completions(ptr, len)` | `target`, `u32 offset`, then a bundle | completion items |
| `xtex_definition(ptr, len)` | `target`, `u32 offset`, then a bundle | the declaration, file included |

The three query exports share one input shape: the target file's name, a byte offset into it, then the
bundle. The table is merged across the whole project, so a name declared in an imported file answers a
query made in the root — and a definition can land in another file, which is why the answer carries the
file. The language server answers from the same core functions over the same project-wide load (the open
buffer overlays the file on disk; imports read from beside it), so the browser and a desktop editor cannot
disagree about one project.

A rename's plan is computed over the whole project and applied one file per call, the way `xtex rename`
writes one file at a time. The plan's `untouched` list is the honest half: every occurrence left alone
because it sits in opaque text, with a position an editor can show. An editor that silently renames 12 of
14 places is worse than one that renames none.

`xtex_blame`'s input is `u32 stderr_len · stderr · u32 log_len · log · bundle` — the engine's console
output and its `.log` file, either of which may be empty. The answer carries, per record: the engine's own
words unchanged, the emitted line, the author's position where a map segment supports one, the entity where
a declaration supplies the evidence, and `"unresolved"` blame otherwise — never a guess. A browser is
exactly where a confident wrong attribution does the most damage, because the user cannot check it against
a terminal.

`xtex_emit` emits the root alone; a host that wants every file's emission calls once per file with that
file as the root.

All the JavaScript it takes:

```js
const { instance } = await WebAssembly.instantiate(bytes, {});
const api = instance.exports;

function call(name, input) {
  const at = api.xtex_alloc(input.length);
  new Uint8Array(api.memory.buffer, at, input.length).set(input);
  const result = api[name](at, input.length);
  const length = new DataView(api.memory.buffer, result, 4).getUint32(0, true);
  const out = new Uint8Array(api.memory.buffer, result + 4, length).slice();
  api.xtex_free_result(result);
  api.xtex_free(at, input.length);
  return out;
}
```

`crates/xtex-wasm/tests/parity.mjs` is that file, and it is what the test runs.

---

## Bytes in, bytes out, and nothing else

The module opens no file, reads no environment variable, and has no notion of a current directory. The
browser has none of those either, and `AGENTS.md` §4 already forbade the core from assuming them — this is
where that constraint is collected on.

Two consequences a caller should expect:

- **Every document is called `document.xtex`.** A browser has no paths; diagnostics still need a file name,
  and inventing a stable one is honest where inventing a path would not be.
- **No bibliography is read**, so the bibliography is `Unavailable` and every `@cite` is silent rather than
  reported as missing. The same is true of the language server, and for the same reason.

Multi-file projects are [#19](https://github.com/camilochs/exacttex/issues/19), where the caller supplies
the store.

---

## The parity test, and what it found

The exit criterion is that the module's output equals the native build's **byte for byte**, on a fixture
chosen to be hostile:

```
\section{Café} @id(sec:caf)\r\n%% comment\t\nSee @ref(sec:caf) and @ref(ghost).\xFF\n
```

A Latin-1 `é`, a CRLF, a tab, and a stray `0xFF` that is not valid UTF-8 anywhere. A boundary that decoded
on the way in or out fails on it, which is the point.

Emitted bytes matched on the first run. **The JSON did not**, and the difference was one digit:

```
native  "coverage":0.4675324675324675
wasm    "coverage":0.4675324675324676
```

Coverage is `1.0 - opaque/total` over byte counts, and on 32-bit `usize` that lands one bit from the 64-bit
result. Sixteen significant digits of a ratio is false precision anyway, and it is not reproducible across
targets, so the JSON now writes six decimals. The diagnostics themselves were identical throughout.

`cargo test -p xtex-wasm` runs the whole comparison. It builds the module, runs it under Node, and compares
against the native path. When the target or Node is missing it says so and returns — a silently skipped test
is worse than an absent one.

---

## Why `unsafe` lives here and nowhere else

The workspace sets `unsafe_code = "forbid"`, which cannot be relaxed from inside a crate. A raw WebAssembly
ABI over caller-provided pointers cannot be written without `unsafe`, so `xtex-wasm` declares its own lint
table instead of inheriting the workspace one. The other four crates keep the forbid.

That is the entire exception, and it is in `crates/xtex-wasm/Cargo.toml` with the reason next to it.
