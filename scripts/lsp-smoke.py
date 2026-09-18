#!/usr/bin/env python3
# LSP smoke test over stdio: python3 scripts/lsp-smoke.py <repo> <file> <line:col>...
# (0-based positions). Prints hover and definition results; the live variant
# exercises completion and diagnostics by editing a class on disk and restoring it.
import json, subprocess, sys, os, time
root = sys.argv.pop(1) if len(sys.argv) > 1 and os.path.isdir(sys.argv[1]) else os.getcwd()
proc = subprocess.Popen(["krab", "lsp"], cwd=root, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=open("/tmp/claude/lsp_stderr.txt", "w"))
nid = 0
def send(method, params, notify=False):
    global nid
    msg = {"jsonrpc": "2.0", "method": method, "params": params}
    if not notify:
        nid += 1; msg["id"] = nid
    body = json.dumps(msg).encode()
    proc.stdin.write(b"Content-Length: %d\r\n\r\n" % len(body) + body); proc.stdin.flush()
    return None if notify else nid
def recv():
    headers = {}
    while True:
        line = proc.stdout.readline()
        if not line: raise SystemExit("server closed: " + open("/tmp/claude/lsp_stderr.txt").read())
        if line == b"\r\n": break
        k, v = line.decode().split(":", 1); headers[k.strip()] = v.strip()
    return json.loads(proc.stdout.read(int(headers["Content-Length"])))
def wait_for(rid):
    while True:
        m = recv()
        if m.get("id") == rid: return m
        if m.get("method") == "textDocument/publishDiagnostics":
            p = m["params"]
            if p["diagnostics"]:
                print("DIAG", os.path.relpath(p["uri"][7:], root), [d["message"][:80] for d in p["diagnostics"]])
send("initialize", {"processId": os.getpid(), "rootUri": "file://" + root, "capabilities": {}})
print("init caps", json.dumps(wait_for(1)["result"]["capabilities"]))
send("initialized", {}, notify=True)
f = sys.argv[1]
uri = "file://" + os.path.join(root, f)
text = open(os.path.join(root, f)).read()
send("textDocument/didOpen", {"textDocument": {"uri": uri, "languageId": "yaml", "version": 1, "text": text}}, notify=True)
for line, col in [tuple(map(int, a.split(":"))) for a in sys.argv[2:]]:
    print(f"\n=== {f}:{line}:{col}  {text.splitlines()[line]!r}")
    t = time.time()
    r = wait_for(send("textDocument/hover", {"textDocument": {"uri": uri}, "position": {"line": line, "character": col}}))
    print(f"--- hover ({(time.time()-t)*1000:.0f} ms)"); print((r.get("result") or {}).get("contents", {}).get("value", r))
    r = wait_for(send("textDocument/definition", {"textDocument": {"uri": uri}, "position": {"line": line, "character": col}}))
    res = r.get("result")
    print("--- definition:", [f"{os.path.relpath(l['uri'][7:], root)}:{l['range']['start']['line']+1}:{l['range']['start']['character']+1}" for l in (res or [])])
if "--complete" in os.environ.get("LSP_EXTRA", ""):
    pass
send("shutdown", {}); wait_for(nid); send("exit", {}, notify=True)
