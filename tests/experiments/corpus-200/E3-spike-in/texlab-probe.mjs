// Copy of exacttex/repo/tests/experiments/0b-error-classes/texlab-probe.mjs, parametrized:
//   node texlab-probe.mjs <root-dir> <root.tex> [wait-ms]
// Opens the root file in texlab, collects every diagnostic published for any file, prints one per line
// as: <SEVERITY> <file> L<line>: <message>. English output; "(no diagnostics)" when none.
import { spawn } from "node:child_process";
import { readFileSync } from "node:fs";
import { resolve } from "node:path";

const dir = resolve(process.argv[2]);
const file = resolve(dir, process.argv[3] ?? "main.tex");
const waitMs = Number(process.argv[4] ?? 8000);
const server = spawn("texlab", [], { stdio: ["pipe", "pipe", "ignore"] });
const send = (msg) => {
  const body = JSON.stringify(msg);
  server.stdin.write(`Content-Length: ${Buffer.byteLength(body)}\r\n\r\n${body}`);
};
let buffer = Buffer.alloc(0);
const diagnostics = new Map(); // uri -> latest list
server.stdout.on("data", (chunk) => {
  buffer = Buffer.concat([buffer, chunk]);
  for (;;) {
    const head = buffer.indexOf("\r\n\r\n");
    if (head < 0) return;
    const length = Number(/Content-Length: (\d+)/.exec(buffer.slice(0, head))?.[1]);
    if (buffer.length < head + 4 + length) return;
    const msg = JSON.parse(buffer.slice(head + 4, head + 4 + length).toString());
    buffer = buffer.slice(head + 4 + length);
    if (msg.method === "textDocument/publishDiagnostics") {
      diagnostics.set(msg.params.uri, msg.params.diagnostics);
    }
  }
});
send({ jsonrpc: "2.0", id: 1, method: "initialize", params: { processId: null, rootUri: `file://${dir}`, capabilities: {} } });
setTimeout(() => {
  send({ jsonrpc: "2.0", method: "initialized", params: {} });
  send({ jsonrpc: "2.0", method: "textDocument/didOpen", params: { textDocument: { uri: `file://${file}`, languageId: "latex", version: 1, text: readFileSync(file, "utf8") } } });
}, 300);
setTimeout(() => {
  const lines = [];
  for (const [uri, ds] of diagnostics) {
    const rel = decodeURIComponent(uri.replace(`file://${dir}/`, ""));
    for (const d of ds) lines.push(`${d.severity === 1 ? "ERROR" : d.severity === 2 ? "WARN" : "info"} ${rel} L${d.range.start.line + 1}: ${d.message}`);
  }
  console.log(lines.length ? lines.join("\n") : "(no diagnostics)");
  server.kill();
  process.exit(0);
}, waitMs);
