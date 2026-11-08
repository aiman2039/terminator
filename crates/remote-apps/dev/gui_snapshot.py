"""Print the remote-app tabs of a running GUI: gui_snapshot.py ROOT"""
import json
import socket
import struct
import sys

root = sys.argv[1]
auth = open(f"{root}/run/auth").read().strip()
body = json.dumps({"version": 1, "auth": auth, "request": "Snapshot"}).encode()
s = socket.socket(socket.AF_UNIX)
s.connect(f"{root}/run/gui.sock")
s.sendall(struct.pack(">I", len(body)) + body)
size = struct.unpack(">I", s.recv(4))[0]
data = b""
while len(data) < size:
    data += s.recv(size - len(data))
reply = json.loads(data)
text = json.dumps(reply)
found = []
def walk(node):
    if isinstance(node, dict):
        if "RemoteApp" in node and isinstance(node["RemoteApp"], dict):
            found.append(node["RemoteApp"])
        for v in node.values():
            walk(v)
    elif isinstance(node, list):
        for v in node:
            walk(v)
walk(reply)
uniq = {(t["window"], t.get("title", "")) for t in found}
print("remote-app tabs:", sorted(uniq))
