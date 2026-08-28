#!/usr/bin/env python3
"""Collect texlab diagnostics for a LaTeX file.

texlab ships no analysis CLI — its only commands are `run` (the language server
over stdin/stdout) and `inverse-search`. To find out what it actually reports,
the server has to be driven over LSP. This script does that: initialize, open
the document, wait for `textDocument/publishDiagnostics`, shut down.

Usage:  python3 texlab_probe.py <file.tex> [seconds]
Output: one JSON object on stdout, {"file": …, "diagnostics": [...]}.
"""

import json
import os
import subprocess
import sys
import threading
import time


def frame(payload: dict) -> bytes:
    body = json.dumps(payload).encode("utf-8")
    return b"Content-Length: %d\r\n\r\n%s" % (len(body), body)


def read_messages(stream, sink, stop):
    """Read Content-Length framed JSON-RPC messages until the stream closes."""
    while not stop.is_set():
        header = b""
        while b"\r\n\r\n" not in header:
            byte = stream.read(1)
            if not byte:
                return
            header += byte
        length = 0
        for line in header.decode("utf-8", "replace").split("\r\n"):
            if line.lower().startswith("content-length:"):
                length = int(line.split(":", 1)[1].strip())
        body = stream.read(length)
        if not body:
            return
        try:
            sink.append(json.loads(body))
        except ValueError:
            pass


def main() -> int:
    if len(sys.argv) < 2:
        print(__doc__, file=sys.stderr)
        return 2
    path = os.path.abspath(sys.argv[1])
    settle = float(sys.argv[2]) if len(sys.argv) > 2 else 4.0
    root = os.path.dirname(path)

    proc = subprocess.Popen(
        ["texlab", "run"],
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=subprocess.DEVNULL,
    )
    received: list = []
    stop = threading.Event()
    reader = threading.Thread(
        target=read_messages, args=(proc.stdout, received, stop), daemon=True
    )
    reader.start()

    def send(payload):
        proc.stdin.write(frame(payload))
        proc.stdin.flush()

    send({
        "jsonrpc": "2.0", "id": 1, "method": "initialize",
        "params": {
            "processId": os.getpid(),
            "rootUri": "file://" + root,
            "capabilities": {
                "textDocument": {"publishDiagnostics": {"relatedInformation": True}}
            },
        },
    })
    time.sleep(1.0)
    send({"jsonrpc": "2.0", "method": "initialized", "params": {}})

    with open(path, "r", encoding="utf-8", errors="replace") as handle:
        text = handle.read()
    send({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": "file://" + path,
                "languageId": "latex",
                "version": 1,
                "text": text,
            }
        },
    })

    time.sleep(settle)
    send({"jsonrpc": "2.0", "id": 2, "method": "shutdown", "params": None})
    time.sleep(0.4)
    send({"jsonrpc": "2.0", "method": "exit", "params": None})
    stop.set()
    try:
        proc.wait(timeout=3)
    except subprocess.TimeoutExpired:
        proc.kill()

    diagnostics = [
        d
        for message in received
        if message.get("method") == "textDocument/publishDiagnostics"
        for d in message.get("params", {}).get("diagnostics", [])
    ]
    print(json.dumps(
        {"file": os.path.basename(path), "diagnostics": diagnostics},
        ensure_ascii=False,
    ))
    return 0


if __name__ == "__main__":
    sys.exit(main())
