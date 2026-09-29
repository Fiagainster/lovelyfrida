#!/usr/bin/env python3
"""M1 端到端联调驱动：以与 Rust 宿主完全相同的方式驱动 frida_bridge.py。

链路：spawn 设置应用 → attach → 加载 agent/core.js → 等 hello → ping/pong → 卸载分离。
用法：python scripts/e2e_test.py [host_port]
"""
import json
import subprocess
import sys
import threading
import queue
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
BRIDGE = ROOT / "sidecar" / "frida_bridge.py"
AGENT = (ROOT / "agent" / "core.js").read_text(encoding="utf-8")

HOST_PORT = int(sys.argv[1]) if len(sys.argv) > 1 else 50005

out_q: "queue.Queue[dict]" = queue.Queue()
proc = subprocess.Popen(
    [sys.executable, "-u", str(BRIDGE)],
    stdin=subprocess.PIPE,
    stdout=subprocess.PIPE,
    text=True,
    encoding="utf-8",
)


def reader():
    for line in proc.stdout:
        line = line.strip()
        if line:
            out_q.put(json.loads(line))


threading.Thread(target=reader, daemon=True).start()


def request(method, params, timeout=20):
    req = {"id": int(time.time() * 1000) % 10**9, "method": method, "params": params}
    proc.stdin.write(json.dumps(req) + "\n")
    proc.stdin.flush()
    deadline = time.time() + timeout
    while time.time() < deadline:
        try:
            msg = out_q.get(timeout=0.2)
        except queue.Empty:
            continue
        if msg.get("type") == "response" and msg.get("id") == req["id"]:
            if msg.get("error"):
                raise RuntimeError(f"{method}: {msg['error']}")
            return msg["result"]
        out_q.put(msg)  # 事件不能丢：放回队列等 wait_event 消费
        time.sleep(0.05)
    raise TimeoutError(method)


def wait_event(name, pred=None, timeout=15):
    deadline = time.time() + timeout
    while time.time() < deadline:
        try:
            msg = out_q.get(timeout=0.2)
        except queue.Empty:
            continue
        if msg.get("type") == "event":
            if msg.get("event") == name and (pred is None or pred(msg.get("params"))):
                return msg["params"]
    raise TimeoutError(f"等待事件 {name} 超时")


def main():
    print("== 1. hello ==")
    info = request("hello", {})
    print("   sidecar:", info)
    assert info["frida"] == "17.19.0", "客户端版本不是 17.19.0"

    print("== 2. remote_connect ==")
    conn = request("remote_connect", {"host": "127.0.0.1", "port": HOST_PORT})
    print("   设备:", conn)

    print("== 3. enumerate_processes ==")
    procs = request("enumerate_processes", {"device": conn["key"]})
    print(f"   共 {len(procs)} 个进程，示例: {[p['name'] for p in procs[:5]]}")

    print("== 4. spawn com.android.settings ==")
    pid = request("spawn", {"device": conn["key"], "program": "com.android.settings"})["pid"]
    print("   pid =", pid)

    print("== 5. attach + create_script + load ==")
    att = request("attach", {"device": conn["key"], "target": pid})
    sid = att["session_id"]
    sc = request("create_script", {"session_id": sid, "source": AGENT, "name": "core"})
    print("   session_id =", sid, "script_id =", sc["script_id"])
    request("load_script", {"script_id": sc["script_id"]})

    print("== 6. 等待 hello（注入成功判据）==")
    hello = wait_event("message", lambda p: p.get("kind") == "send" and (p.get("payload") or {}).get("t") == "hello")
    print("   hello:", hello["payload"])

    print("== 7. resume ==")
    request("resume", {"device": conn["key"], "pid": pid})

    print("== 8. ping → pong 消息回路 ==")
    request("post", {"script_id": sc["script_id"], "message": {"type": "ping", "data": {"x": 1}}})
    pong = wait_event("message", lambda p: p.get("kind") == "send" and (p.get("payload") or {}).get("t") == "pong")
    print("   pong:", pong["payload"])

    print("== 9. 卸载 + 分离 ==")
    request("unload_script", {"script_id": sc["script_id"]})
    request("detach", {"session_id": sid})
    request("kill", {"device": conn["key"], "pid": pid})
    print("\n✅ 端到端联调全部通过")


if __name__ == "__main__":
    try:
        main()
    finally:
        proc.terminate()
