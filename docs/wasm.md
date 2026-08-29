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
2. Copy the document there.
3. Call an operation with `(pointer, len)`. It returns a **result pointer**: four little-endian bytes of
   length, then that many bytes.
4. Read them, then `xtex_free_result(result)` and `xtex_free(pointer, len)`.

The length prefix is not decoration. Emitted LaTeX can contain a zero byte, and a C-string convention would
truncate the document at it.

| Export | Returns |
|---|---|
| `xtex_alloc(len)` | a pointer to writable bytes |
| `xtex_free(ptr, len)` | — |
| `xtex_free_result(ptr)` | — |
| `xtex_emit(ptr, len)` | the emitted LaTeX |
| `xtex_check_json(ptr, len)` | the JSON `xtex check --json` prints |
| `xtex_source_map(ptr, len)` | the source map, as JSON |

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
