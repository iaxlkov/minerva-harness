import json,sys,os,pathlib,urllib.parse,urllib.request
while True:
 headers={}
 while True:
  line=sys.stdin.buffer.readline()
  if not line: sys.exit(0)
  if line in (b"\r\n",b"\n"): break
  key,value=line.decode().split(":",1);headers[key.lower()]=value.strip()
 msg=json.loads(sys.stdin.buffer.read(int(headers["content-length"])))
 method=msg.get("method")
 if method=="exit": break
 if method=="initialized":
  data=json.dumps({"jsonrpc":"2.0","id":"config-check","method":"workspace/configuration","params":{"items":[{"section":"test"}]}}).encode()
  sys.stdout.buffer.write(f"Content-Length: {len(data)}\r\n\r\n".encode()+data);sys.stdout.buffer.flush()
  continue
 if method is None:
  if msg.get("id")=="config-check" and msg.get("result")==[None]:
   data=json.dumps({"jsonrpc":"2.0","method":"textDocument/publishDiagnostics","params":{"uri":pathlib.Path("proof.rs").resolve().as_uri(),"diagnostics":[]}}).encode()
   sys.stdout.buffer.write(f"Content-Length: {len(data)}\r\n\r\n".encode()+data);sys.stdout.buffer.flush()
  continue
 if "id" not in msg: continue
 if method=="initialize":
  folders=msg["params"].get("workspaceFolders") or []
  if not folders: raise RuntimeError("workspaceFolders missing")
  uri=urllib.parse.urlsplit(folders[0]["uri"])
  root=pathlib.Path(urllib.request.url2pathname(uri.path)).resolve()
  if root!=pathlib.Path.cwd().resolve(): raise RuntimeError(f"root mismatch: {root} != {pathlib.Path.cwd()}")
  result={"capabilities":{},"serverInfo":{"name":os.getcwd(),"version":folders[0]["uri"] if folders else "missing"}}
 else: result=None
 data=json.dumps({"jsonrpc":"2.0","id":msg["id"],"result":result}).encode();sys.stdout.buffer.write(f"Content-Length: {len(data)}\r\n\r\n".encode()+data);sys.stdout.buffer.flush()
