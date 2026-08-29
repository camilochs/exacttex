// Runs the WebAssembly module and prints what it produced, so the harness can
// compare it against the native tool byte for byte.
//
// No bundler and no generated glue: the module is a `.wasm` file with six
// exports and a linear memory, and this is all the JavaScript it takes —
// including the project bundle, which is a DataView and a loop.
import { readFileSync, writeFileSync, readdirSync, statSync } from "node:fs";
import { join, relative } from "node:path";

const [, , wasmPath, projectDir, rootName, outDir] = process.argv;
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

// The bundle: u32 root_len, root, u32 count, then (u32 name_len, name,
// u32 data_len, data) per file. Everything little-endian, nothing aligned.
function bundle(dir, root) {
  const files = [];
  const walk = (d) => {
    for (const entry of readdirSync(d)) {
      const path = join(d, entry);
      if (statSync(path).isDirectory()) walk(path);
      else files.push([relative(dir, path).split("\\").join("/"), readFileSync(path)]);
    }
  };
  walk(dir);
  files.sort((a, b) => (a[0] < b[0] ? -1 : 1));

  const rootBytes = new TextEncoder().encode(root);
  let size = 4 + rootBytes.length + 4;
  const encoded = files.map(([name, data]) => {
    const nameBytes = new TextEncoder().encode(name);
    size += 4 + nameBytes.length + 4 + data.length;
    return [nameBytes, data];
  });
  const out = new Uint8Array(size);
  const view = new DataView(out.buffer);
  let at = 0;
  const u32 = (n) => { view.setUint32(at, n, true); at += 4; };
  const put = (bytes) => { out.set(bytes, at); at += bytes.length; };
  u32(rootBytes.length); put(rootBytes);
  u32(encoded.length);
  for (const [name, data] of encoded) {
    u32(name.length); put(name);
    u32(data.length); put(data);
  }
  return out;
}

const project = bundle(projectDir, rootName);
writeFileSync(`${outDir}/wasm.tex`, call("xtex_emit", project));
writeFileSync(`${outDir}/wasm.json`, call("xtex_check_json", project));
writeFileSync(`${outDir}/wasm.map`, call("xtex_source_map", project));

// Helper: length-prefixed texts followed by the bundle.
function framed(texts, bundleBytes) {
  const enc = new TextEncoder();
  const encoded = texts.map((t) => (t instanceof Uint8Array ? t : enc.encode(t)));
  let size = bundleBytes.length;
  for (const t of encoded) size += 4 + t.length;
  const out = new Uint8Array(size);
  const view = new DataView(out.buffer);
  let at = 0;
  for (const t of encoded) {
    view.setUint32(at, t.length, true); at += 4;
    out.set(t, at); at += t.length;
  }
  out.set(bundleBytes, at);
  return out;
}

// Rename: plan JSON and the root's rewritten bytes.
writeFileSync(`${outDir}/wasm.rename.json`, call("xtex_rename_plan", framed(["sec:model", "sec:modelo"], project)));
writeFileSync(`${outDir}/wasm.renamed.root`, call("xtex_rename_apply", framed(["sec:model", "sec:modelo", rootName], project)));

// Positional queries: target file, u32 byte offset, then the bundle.
function positional(target, offset, bundleBytes) {
  const enc = new TextEncoder();
  const t = enc.encode(target);
  const out = new Uint8Array(4 + t.length + 4 + bundleBytes.length);
  const view = new DataView(out.buffer);
  let at = 0;
  view.setUint32(at, t.length, true); at += 4;
  out.set(t, at); at += t.length;
  view.setUint32(at, offset, true); at += 4;
  out.set(bundleBytes, at);
  return out;
}
const rootText = readFileSync(join(projectDir, rootName), "utf8");
const refAt = rootText.indexOf("@ref(sec:model)") + 6;
const citeAt = rootText.indexOf("@cite(knuth1984)") + 7;
writeFileSync(`${outDir}/wasm.hover.json`, call("xtex_hover", positional(rootName, refAt, project)));
writeFileSync(`${outDir}/wasm.completions.json`, call("xtex_completions", positional(rootName, refAt, project)));
writeFileSync(`${outDir}/wasm.definition.json`, call("xtex_definition", positional(rootName, refAt, project)));
writeFileSync(`${outDir}/wasm.hover.opaque.json`, call("xtex_hover", positional(rootName, rootText.indexOf("verb|sec:model") + 6, project)));
writeFileSync(`${outDir}/wasm.hover.pastend.json`, call("xtex_hover", positional(rootName, 10_000_000, project)));
writeFileSync(`${outDir}/wasm.hover.cite.json`, call("xtex_hover", positional(rootName, citeAt, project)));

// Revision views and one accept, over the revisions fixture when present.
const revDir = join(projectDir, "../revisions");
try {
  const revBundle = bundle(revDir, "paper.xtex");
  for (const view of ["original", "final", "marked"]) {
    writeFileSync(`${outDir}/wasm.view.${view}.tex`, call("xtex_view", framed([view], revBundle)));
  }
  const sidecar = readFileSync(join(revDir, "paper.xtexrev"));
  const accepted = call(
    "xtex_revise",
    framed(["accept", "c1", "browser-reviewer", "2026-08-30T10:00:00Z", sidecar], revBundle),
  );
  writeFileSync(`${outDir}/wasm.revise.pair`, accepted);
} catch {}

// Optional: a TeX log to translate. Two length-prefixed texts, then the bundle.
const [, , , , , , stderrPath, logPath] = process.argv;
if (stderrPath && logPath) {
  const stderrBytes = readFileSync(stderrPath);
  const logBytes = readFileSync(logPath);
  writeFileSync(`${outDir}/wasm.blame.json`, call("xtex_blame", framed([stderrBytes, logBytes], project)));
}
