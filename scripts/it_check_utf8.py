#!/usr/bin/env python3
"""验证 sidecar exe 的 stdout 是否存在非 UTF-8 字节（宿主读循环死因）。"""
import json
import subprocess
import threading
import time

p = subprocess.Popen(
    [r"sidecar\dist\frida_bridge.exe"], stdin=subprocess.PIPE, stdout=subprocess.PIPE,
    stderr=subprocess.PIPE, creationflags=0x08000000)
findings = []
done = threading.Event()


def rd():
    n = 0
    while True:
        b = p.stdout.readline()
        if not b:
            findings.append(("EOF", b""))
            done.set()
            break
        n += 1
        try:
            b.decode("utf-8")
            if any(x > 127 for x in b):
                findings.append((f"line{n} non-ascii-but-utf8", b[:160]))
        except UnicodeDecodeError as e:
            findings.append((f"line{n} *** NOT-UTF8 *** at {e.start}", b[max(0, e.start - 20):e.start + 40]))
        if n >= 2:
            done.set()


threading.Thread(target=rd, daemon=True).start()


def send(o):
    p.stdin.write(json.dumps(o).encode("utf-8"))
    p.stdin.flush()


time.sleep(2)
send({"id": 1, "method": "remote_connect", "params": {"host": "127.0.0.1", "port": 27081}})
time.sleep(3)
send({"id": 2, "method": "enumerate_processes", "params": {"device": "127.0.0.1:27081"}})
done.wait(20)
time.sleep(1)
for f in findings[:8]:
    print(f[0], f[1])
p.kill()
