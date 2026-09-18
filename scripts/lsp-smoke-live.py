#!/usr/bin/env python3
# LSP smoke test over stdio: python3 scripts/lsp-smoke.py <repo> <file> <line:col>...
# (0-based positions). Prints hover and definition results; the live variant
# exercises completion and diagnostics by editing a class on disk and restoring it.
# completion + live diagnostics
import json, subprocess, sys, os, time
root = sys.argv.pop(1) if len(sys.argv) > 1 and os.path.isdir(sys.argv[1]) else os.getcwd()
proc = subprocess.Popen(["krab", "lsp"], cwd=root, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=open("/tmp/claude/lsp_stderr2.txt", "w"))
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
        if not line: raise SystemExit("server closed: " + open("/tmp/claude/lsp_stderr2.txt").read())
        if line == b"\r\n": break
        k, v = line.decode().split(":", 1); headers[k.strip()] = v.strip()
    return json.loads(proc.stdout.read(int(headers["Content-Length"])))
diags = {}
def pump(rid=None, seconds=0):
    end = time.time() + seconds
    import select
    while True:
        if rid is None and time.time() > end: return None
        r, _, _ = select.select([proc.stdout], [], [], 0.2)
        if not r: 
            if rid is None: continue
            continue
        m = recv()
        if m.get("method") == "textDocument/publishDiagnostics":
            p = m["params"]; rel = os.path.relpath(p["uri"][7:], root)
            diags[rel] = p["diagnostics"]
            print(f"DIAG {rel}: {[ (d['range']['start']['line']+1, d['source'], d['message'].splitlines()[0][:100]) for d in p['diagnostics']] or 'cleared'}")
        elif rid is not None and m.get("id") == rid: return m
send("initialize", {"processId": os.getpid(), "rootUri": "file://" + root, "capabilities": {}}); pump(1)
send("initialized", {}, notify=True)
f = "inventory/targets/aws/eu-central-1/cluster.yml"
uri = "file://" + os.path.join(root, f)
text = open(os.path.join(root, f)).read()
send("textDocument/didOpen", {"textDocument": {"uri": uri, "languageId": "yaml", "version": 1, "text": text}}, notify=True)
# completion of a parameter path: edit buffer to add "${cluster." at line 21
lines = text.splitlines()
lines.insert(21, "      probe: ${cluster.")
send("textDocument/didChange", {"textDocument": {"uri": uri, "version": 2}, "contentChanges": [{"text": "\n".join(lines) + "\n"}]}, notify=True)
t=time.time(); r = pump(send("textDocument/completion", {"textDocument": {"uri": uri}, "position": {"line": 21, "character": len("      probe: ${cluster.")}}))
print(f"--- completion ${{cluster. ({(time.time()-t)*1000:.0f} ms):", [(i["label"], i["detail"]) for i in r["result"]][:12])
r = pump(send("textDocument/completion", {"textDocument": {"uri": uri}, "position": {"line": 21, "character": len("      probe: ${")}}))
print("--- completion ${ :", len(r["result"]), "items, e.g.", [i["label"] for i in r["result"]][:8])
# completion of a class name in a new item
lines = text.splitlines(); lines.insert(2, "  - components.gra")
send("textDocument/didChange", {"textDocument": {"uri": uri, "version": 3}, "contentChanges": [{"text": "\n".join(lines) + "\n"}]}, notify=True)
t=time.time(); r = pump(send("textDocument/completion", {"textDocument": {"uri": uri}, "position": {"line": 2, "character": len("  - components.gra")}}))
print(f"--- class completion ({(time.time()-t)*1000:.0f} ms):", len(r["result"]), "items, e.g.", [i["label"] for i in r["result"] if i["label"].startswith("components.grafana")][:5])
# live diagnostics: break a class on disk, wait for publish, restore
cls = os.path.join(root, "inventory/classes/roles/kamaji-tenant/cluster.yml")
orig = open(cls).read()
print("--- breaking class file on disk")
open(cls, "w").write(orig.replace("parameters:", "classes:\n  - does.not.exist\nparameters:", 1))
try:
    t=time.time(); pump(seconds=6); print(f"    (waited {time.time()-t:.1f}s)")
finally:
    open(cls, "w").write(orig)
print("--- restored"); pump(seconds=6)
send("shutdown", {}); pump(nid); send("exit", {}, notify=True)
