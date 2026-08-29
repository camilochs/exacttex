// Runs the WebAssembly module and prints what it produced, so the harness can
// compare it against the native tool byte for byte.
//
// No bundler and no generated glue: the module is a `.wasm` file with six
// exports and a linear memory, and this is all the JavaScript it takes.
import { readFileSync, writeFileSync } from "node:fs";

const [, , wasmPath, inputPath, outDir] = process.argv;
const { instance } = await WebAssembly.instantiate(readFileSync(wasmPath), {});
const api = instance.exports;

function call(name, bytes) {
  const input = api.xtex_alloc(bytes.length);
  new Uint8Array(api.memory.buffer, input, bytes.length).set(bytes);
  const result = api[name](input, bytes.length);
  // Four little-endian bytes of length, then that many bytes.
  const header = new DataView(api.memory.buffer, result, 4);
  const length = header.getUint32(0, true);
  const out = new Uint8Array(api.memory.buffer, result + 4, length).slice();
  api.xtex_free_result(result);
  api.xtex_free(input, bytes.length);
  return out;
}

const source = readFileSync(inputPath);
writeFileSync(`${outDir}/wasm.tex`, call("xtex_emit", source));
writeFileSync(`${outDir}/wasm.json`, call("xtex_check_json", source));
writeFileSync(`${outDir}/wasm.map`, call("xtex_source_map", source));
