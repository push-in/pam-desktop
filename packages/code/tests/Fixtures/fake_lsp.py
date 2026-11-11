# A tiny language server for the client tests: answers initialize, hover,
# completion and definition, and publishes one diagnostic per opened file.
import json, sys

def read():
    length = 0
    while True:
        line = sys.stdin.buffer.readline()
        if not line:
            sys.exit(0)
        line = line.decode().strip()
        if not line:
            break
        if line.lower().startswith("content-length:"):
            length = int(line.split(":")[1])
    return json.loads(sys.stdin.buffer.read(length))

def send(message):
    body = json.dumps(message).encode()
    sys.stdout.buffer.write(b"Content-Length: %d\r\n\r\n" % len(body) + body)
    sys.stdout.buffer.flush()

while True:
    message = read()
    method = message.get("method")
    if method == "initialize":
        send({"jsonrpc": "2.0", "id": message["id"], "result": {"capabilities": {"hoverProvider": True}}})
        send({"jsonrpc": "2.0", "id": 900, "method": "workspace/configuration", "params": {"items": [{}, {}]}})
    elif method == "textDocument/didOpen":
        uri = message["params"]["textDocument"]["uri"]
        send({"jsonrpc": "2.0", "method": "textDocument/publishDiagnostics", "params": {"uri": uri, "diagnostics": [
            {"range": {"start": {"line": 1, "character": 4}, "end": {"line": 1, "character": 9}}, "severity": 2, "message": "unused variable", "source": "fake"}]}})
    elif method == "textDocument/hover":
        position = message["params"]["position"]
        send({"jsonrpc": "2.0", "id": message["id"], "result": {"contents": {"kind": "markdown", "value": "**fn main** at %d:%d" % (position["line"], position["character"])}}})
    elif method == "textDocument/completion":
        send({"jsonrpc": "2.0", "id": message["id"], "result": {"isIncomplete": False, "items": [{"label": "println!", "kind": 3, "detail": "macro"}, {"label": "print!", "insertText": "print!()"}]}})
    elif method == "textDocument/definition":
        send({"jsonrpc": "2.0", "id": message["id"], "result": [{"uri": message["params"]["textDocument"]["uri"], "range": {"start": {"line": 7, "character": 3}, "end": {"line": 7, "character": 8}}}]})
    elif method == "shutdown":
        send({"jsonrpc": "2.0", "id": message["id"], "result": None})
    elif method == "exit":
        sys.exit(0)
    elif "id" in message and method == "nope":
        send({"jsonrpc": "2.0", "id": message["id"], "error": {"code": -32601, "message": "no such method"}})
    elif "id" in message and method is None:
        pass  # our answer to workspace/configuration
