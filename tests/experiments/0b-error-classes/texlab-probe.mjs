// Speaks just enough LSP to texlab: open the case's main.tex, collect
// every diagnostic it publishes for a few seconds, print them.
import { spawn } from "node:child_process";
import { readFileSync } from "node:fs";
import { resolve } from "node:path";

const dir = resolve(process.argv[2]);
const file = resolve(dir, "main.tex");
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
    if (msg.method === "textDocument/publishDiagnostics" && msg.params.diagnostics.length) {
      for (const d of msg.params.diagnostics) diagnostics.push(`${d.severity === 1 ? "ERROR" : d.severity === 2 ? "WARN" : "info"} L${d.range.start.line + 1}: ${d.message}`);
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
