import json, sys, os, pathlib, urllib.parse, urllib.request

documents = {}
def send(message):
    data = json.dumps(message).encode()
    sys.stdout.buffer.write(f"Content-Length: {len(data)}\r\n\r\n".encode() + data)
    sys.stdout.buffer.flush()
while True:
    headers = {}
    while True:
        line = sys.stdin.buffer.readline()
        if not line: sys.exit(0)
        if line in (b"\r\n", b"\n"): break
        key, value = line.decode().split(":", 1)
        headers[key.lower()] = value.strip()
    msg = json.loads(sys.stdin.buffer.read(int(headers["content-length"])))
    method = msg.get("method")
    if method == "exit": break
    if method == "initialized":
        send({"jsonrpc":"2.0", "id":"config-check", "method":"workspace/configuration", "params":{"items":[{"section":"test"}]}})
        continue
    if method is None:
        if msg.get("id") == "config-check" and msg.get("result") == [None]:
            send({"jsonrpc":"2.0", "method":"textDocument/publishDiagnostics", "params":{"uri":pathlib.Path("proof.rs").resolve().as_uri(), "diagnostics":[]}})
        continue
    if method == "textDocument/didOpen":
        doc = msg["params"]["textDocument"]
        documents[doc["uri"]] = doc["text"]
        send({"jsonrpc":"2.0", "method":"textDocument/publishDiagnostics", "params":{"uri":doc["uri"], "version":doc["version"], "diagnostics":[]}})
        continue
    if method == "textDocument/didChange":
        doc = msg["params"]["textDocument"]
        documents[doc["uri"]] = msg["params"]["contentChanges"][0]["text"]
        continue
    if method == "textDocument/didClose":
        documents.pop(msg["params"]["textDocument"]["uri"], None)
        continue
    if "id" not in msg: continue
    if method == "initialize":
        folders = msg["params"].get("workspaceFolders") or []
        if not folders: raise RuntimeError("workspaceFolders missing")
        uri = urllib.parse.urlsplit(folders[0]["uri"])
        root = pathlib.Path(urllib.request.url2pathname(uri.path)).resolve()
        if root != pathlib.Path.cwd().resolve(): raise RuntimeError("root mismatch")
        result = {"capabilities":{}, "serverInfo":{"name":os.getcwd(), "version":folders[0]["uri"]}}
    elif method == "textDocument/hover":
        result = {"contents":documents[msg["params"]["textDocument"]["uri"]]}
    else: result = None
    send({"jsonrpc":"2.0", "id":msg["id"], "result":result})
