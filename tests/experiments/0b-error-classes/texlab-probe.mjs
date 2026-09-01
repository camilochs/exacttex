// Speaks just enough LSP to texlab: open the case's main.tex (or the stem
// named as the second argument, e.g. `clean`), collect every diagnostic it
// publishes for that file for a few seconds, print them once each.
//   node texlab-probe.mjs cases/<name> [main|clean]
//
// texlab publishes diagnostics for every .tex it finds under the workspace,
// and a case directory holds two roots (main.tex and clean.tex), so only
// the opened file's diagnostics are kept — without the filter, probing the
// clean twin printed the defective twin's findings under its name.
import { spawn } from "node:child_process";
import { readFileSync } from "node:fs";
import { resolve } from "node:path";

const dir = resolve(process.argv[2]);
const file = resolve(dir, `${process.argv[3] ?? "main"}.tex`);
const server = spawn("texlab", [], { stdio: ["pipe", "pipe", "ignore"] });
const send = (msg) => {
  const body = JSON.stringify(msg);
  server.stdin.write(`Content-Length: ${Buffer.byteLength(body)}\r\n\r\n${body}`);
};
let buffer = Buffer.alloc(0);
const diagnostics = [];
server.stdout.on("data", (chunk) => {
  buffer = Buffer.concat([buffer, chunk]);
  for (;;) {
    const head = buffer.indexOf("\r\n\r\n");
    if (head < 0) return;
    const length = Number(/Content-Length: (\d+)/.exec(buffer.slice(0, head))?.[1]);
    if (buffer.length < head + 4 + length) return;
    const msg = JSON.parse(buffer.slice(head + 4, head + 4 + length).toString());
    buffer = buffer.slice(head + 4 + length);
    if (msg.method === "textDocument/publishDiagnostics" && msg.params.uri === `file://${file}`) {
      for (const d of msg.params.diagnostics) {
        const line = `${d.severity === 1 ? "ERROR" : d.severity === 2 ? "WARN" : "info"} L${d.range.start.line + 1}: ${d.message}`;
        if (!diagnostics.includes(line)) diagnostics.push(line);
      }
    }
  }
});
send({ jsonrpc: "2.0", id: 1, method: "initialize", params: { processId: null, rootUri: `file://${dir}`, capabilities: {} } });
setTimeout(() => {
  send({ jsonrpc: "2.0", method: "initialized", params: {} });
  send({ jsonrpc: "2.0", method: "textDocument/didOpen", params: { textDocument: { uri: `file://${file}`, languageId: "latex", version: 1, text: readFileSync(file, "utf8") } } });
}, 300);
setTimeout(() => {
  console.log(diagnostics.length ? diagnostics.join("\n") : "(sin diagnosticos)");
  server.kill();
  process.exit(0);
}, 4000);
