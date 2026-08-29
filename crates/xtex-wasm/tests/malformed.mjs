// Feeds the module bundles that lie about their lengths, and asserts every
// answer is the empty result rather than a trap. Exit criterion 4 of #69:
// no out-of-bounds read, no panic crossing the ABI boundary.
import { readFileSync } from "node:fs";

const [, , wasmPath] = process.argv;
const { instance } = await WebAssembly.instantiate(readFileSync(wasmPath), {});
const api = instance.exports;

function callExpectEmpty(name, bytes, label) {
  const input = api.xtex_alloc(bytes.length);
  new Uint8Array(api.memory.buffer, input, bytes.length).set(bytes);
  const result = api[name](input, bytes.length);
  const header = new DataView(api.memory.buffer, result, 4);
  const length = header.getUint32(0, true);
  api.xtex_free_result(result);
  api.xtex_free(input, bytes.length);
  if (length !== 0) {
    console.error(`${label}: expected an empty result, got ${length} bytes`);
    process.exit(1);
  }
}

// A bundle is built valid first, then corrupted, so that a decoder which
// shrugs at the corruption would go on to CHECK A WORKING PROJECT and
// return a non-empty answer. A case whose project fails anyway proves
// nothing about the decoder — the first version of this file did exactly
// that, and two deliberate decoder bugs sailed through it.
function validBundle(root, files) {
  const enc = new TextEncoder();
  const rootBytes = enc.encode(root);
  let size = 4 + rootBytes.length + 4;
  const encoded = files.map(([n, d]) => [enc.encode(n), enc.encode(d)]);
  for (const [n, d] of encoded) size += 8 + n.length + d.length;
  const out = new Uint8Array(size);
  const view = new DataView(out.buffer);
  let at = 0;
  const u32 = (x) => { view.setUint32(at, x, true); at += 4; };
  const put = (b) => { out.set(b, at); at += b.length; };
  u32(rootBytes.length); put(rootBytes);
  u32(encoded.length);
  for (const [n, d] of encoded) { u32(n.length); put(n); u32(d.length); put(d); }
  return out;
}

const good = validBundle("a.xtex", [["a.xtex", "hola @id(x:a)"]]);

// Sanity: the valid bundle must produce a non-empty answer, or every case
// below passes vacuously.
{
  const input = api.xtex_alloc(good.length);
  new Uint8Array(api.memory.buffer, input, good.length).set(good);
  const result = api.xtex_check_json(input, good.length);
  const length = new DataView(api.memory.buffer, result, 4).getUint32(0, true);
  api.xtex_free_result(result);
  api.xtex_free(input, good.length);
  if (length === 0) {
    console.error("the valid control bundle produced nothing; the cases prove nothing");
    process.exit(1);
  }
}

const trailing = new Uint8Array(good.length + 1);
trailing.set(good); trailing[good.length] = 9;

// data_len inflated far past the end: a decoder that clamps instead of
// refusing still finds the root and answers.
const inflated = good.slice();
{
  const view = new DataView(inflated.buffer);
  // last u32 written is the file's data_len, at offset: 4+6 +4 +4+6 = 24
  view.setUint32(24, 0xffffffff, true);
}

const cases = [
  ["empty", new Uint8Array(0)],
  ["root overruns", new Uint8Array([100, 0, 0, 0, 65, 66])],
  ["count overruns", new Uint8Array([1, 0, 0, 0, 97, 1, 0, 0, 0])],
  ["data_len overflows on a working project", inflated],
  ["trailing byte after a working project", trailing],
  ["root not UTF-8", new Uint8Array([1, 0, 0, 0, 255, 0, 0, 0, 0])],
];

for (const [label, bytes] of cases) {
  for (const name of ["xtex_emit", "xtex_check_json", "xtex_source_map"]) {
    callExpectEmpty(name, bytes, `${name} / ${label}`);
  }
}
console.log("every malformed bundle answered empty");
