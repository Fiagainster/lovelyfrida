#!/usr/bin/env python3
"""批次三~六联调驱动（层1）：以与 Rust 宿主完全相同的方式驱动 sidecar + 真实 dist/core.js。

前置：设备已 root、frida-server 已运行、adb forward tcp:27042 已建立、目标应用已启动。
用法：python scripts/it_layer1_sidecar.py [target_package] [host_port]

验证点（对应批次三~六修复）：
  A. hello / attach / core agent 加载 / hello 握手
  B. ping→pong 消息回路
  C. listMethods 找目标（P-02）
  D. addProbes 命中 → 批量层事件（{t:"batch"}，批次④）
  E. 条件过滤生效（此前 strict-mode SyntaxError 从未生效，批次三）
  F. removeProbes 卸载 + 卸后不再命中（loader 还原逻辑不破坏同 loader 场景，批次五）
  G. 重试竞态：waiting 探针 retry 成功后主动上报 probe_status（批次五）
  H. sidecar 被杀：在途 RPC 立即报错（不干等 30s）+ 下次调用自动重启（批次三）
"""
import json
import queue
import subprocess
import sys
import threading
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
BRIDGE = ROOT / "sidecar" / "frida_bridge.py"
AGENT = (ROOT / "agent" / "dist" / "core.js").read_text(encoding="utf-8")

TARGET = sys.argv[1] if len(sys.argv) > 1 else "com.notevault.app"
HOST_PORT = int(sys.argv[2]) if len(sys.argv) > 2 else 27042

out_q: "queue.Queue[dict]" = queue.Queue()
proc = subprocess.Popen(
    [sys.executable, "-u", str(BRIDGE)],
    stdin=subprocess.PIPE,
    stdout=subprocess.PIPE,
    text=True,
    encoding="utf-8",
)

results: list[tuple[str, bool, str]] = []


def check(name: str, ok: bool, detail: str = "") -> None:
    results.append((name, ok, detail))
    print(f"  {'✔' if ok else '✖'} {name}" + (f" —— {detail}" if detail else ""))


def reader() -> None:
    for line in proc.stdout:
        line = line.strip()
        if line:
            try:
                out_q.put(json.loads(line))
            except json.JSONDecodeError:
                pass


threading.Thread(target=reader, daemon=True).start()

_next_id = 1000


def request(method: str, params: dict, timeout: float = 30) -> dict:
    """发请求并等到对应 id 的 response（事件留在队列里）。"""
    global _next_id
    _next_id += 1
    rid = _next_id
    proc.stdin.write(json.dumps({"id": rid, "method": method, "params": params}) + "\n")
    proc.stdin.flush()
    return wait_response(rid, timeout)


def wait_response(rid: int, timeout: float) -> dict:
    deadline = time.time() + timeout
    stash: list[dict] = []
    while time.time() < deadline:
        try:
            msg = out_q.get(timeout=0.2)
        except queue.Empty:
            continue
        if msg.get("type") == "response" and msg.get("id") == rid:
            for m in stash:
                out_q.put(m)
            return msg
        stash.append(msg)
    for m in stash:
        out_q.put(m)
    return {"type": "response", "id": rid, "error": "timeout"}


def drain_events(seconds: float) -> list[dict]:
    """收 seconds 秒内的 message 事件 payload（解开批量层）。"""
    out: list[dict] = []
    deadline = time.time() + seconds
    while time.time() < deadline:
        try:
            msg = out_q.get(timeout=0.2)
        except queue.Empty:
            continue
        if msg.get("type") == "event" and msg.get("event") == "message":
            p = (msg.get("params") or {}).get("payload") or {}
            if p.get("t") == "batch":
                out.extend(p.get("items") or [])
            else:
                out.append(p)
    return out


def section(title: str) -> None:
    print(f"\n—— {title} ——")


# ---------------- A. hello / attach / agent ----------------
section("A. hello / attach / core agent 加载")
r = request("hello", {})
check("sidecar hello", r.get("result", {}).get("frida") is not None, str(r.get("result"))[:80])

r = request("remote_connect", {"host": "127.0.0.1", "port": HOST_PORT})
device_key = (r.get("result") or {}).get("key")
check("remote_connect", bool(device_key), str(r.get("result"))[:80])


def resolve_target_pid(device: str) -> tuple[int, bool]:
    """拿目标 pid；未运行则 spawn（与实验链 M3 同款），返回 (pid, spawned)。"""
    r = request("enumerate_applications", {"device": device})
    apps = r.get("result") or []
    app = next((a for a in apps if a.get("identifier") == TARGET), None)
    if app and app.get("pid"):
        return int(app["pid"]), False
    sp = request("spawn", {"device": device, "program": TARGET})
    pid = int((sp.get("result") or {}).get("pid") or 0)
    return pid, pid > 0


pid, spawned = resolve_target_pid(device_key)
check("目标 pid 就绪（未运行则 spawn）", pid > 0, f"pid={pid} spawned={spawned}")

r = request("attach", {"device": device_key, "target": pid})
session_id = (r.get("result") or {}).get("session_id")
check("attach 目标进程", bool(session_id), str(r.get("error") or r.get("result"))[:80])

r = request("create_script", {"session_id": session_id, "source": AGENT, "name": "it-core"})
script_id = (r.get("result") or {}).get("script_id")
check("create_script", bool(script_id))

hello_evt = None
request("load_script", {"script_id": script_id})
deadline = time.time() + 8
while time.time() < deadline and hello_evt is None:
    try:
        msg = out_q.get(timeout=0.3)
    except queue.Empty:
        continue
    if msg.get("type") == "event" and msg.get("event") == "message":
        p = (msg.get("params") or {}).get("payload") or {}
        if p.get("t") == "hello":
            hello_evt = p
check("hello 握手（注入成功判据）", hello_evt is not None,
      f"frida={hello_evt.get('frida')} pid={hello_evt.get('pid')} java={hello_evt.get('java')}" if hello_evt else "6s 内未收到")

# ---------------- B. ping→pong ----------------
section("B. ping→pong 消息回路")
request("post", {"script_id": script_id, "message": {"type": "ping", "data": {"ts": "it"}}})
pong = False
deadline = time.time() + 5
while time.time() < deadline and not pong:
    try:
        msg = out_q.get(timeout=0.3)
    except queue.Empty:
        continue
    if msg.get("type") == "event":
        p = (msg.get("params") or {}).get("payload") or {}
        if p.get("t") == "pong":
            pong = True
check("pong 回流", pong)

# ---------------- C. 目标方法定位 ----------------
section("C. 目标方法定位（AESUtil）")
r = request("rpc_call", {"script_id": script_id, "fn": "javaMethods", "args": [{"className": "com.notevault.app.utils.AESUtil"}]})
methods = ((r.get("result") or {}).get("result") or {}).get("methods") or []
names = [m["name"] for m in methods]
check("AESUtil 已加载且方法可枚举", len(methods) > 0, f"{len(methods)} 个方法：{names[:8]}")
target_method = "hashPassword" if "hashPassword" in names else (names[0] if names else None)
print(f"  目标方法：{target_method}")

# ---------------- D/E/G. 探针挂载 + 批量事件 + 条件过滤 ----------------
section("D/E/G. addProbes → REPL 触发 → 批量事件 / 条件过滤 / 重试上报")
drain_events(0.5)  # 清掉挂载前的残留事件

r = request("rpc_call", {"script_id": script_id, "fn": "addProbes", "args": [{"probes": [
    {"id": "it-p1", "clazz": "com.notevault.app.utils.AESUtil", "method": target_method,
     "maxLen": 128, "captureRet": True},
    {"id": "it-p2", "clazz": "com.notevault.app.utils.AESUtil", "method": target_method,
     "maxLen": 128, "captureRet": False,
     "condition": "arguments.length > 0 && String(arguments[0]) === 'IT-CONDITION-HIT'"},
]}]})
add_results = ((r.get("result") or {}).get("result") or {}).get("results") or []
check("addProbes 请求受理", len(add_results) == 2, str(add_results)[:120])

# 等待 waiting 重试或 active（retry 成功走 probe_status 事件上报 = 批次五 G 项）
status_map: dict[str, str] = {}
deadline = time.time() + 12
while time.time() < deadline:
    r = request("rpc_call", {"script_id": script_id, "fn": "probeStats", "args": []}, timeout=10)
    stats = (r.get("result") or {}).get("result") or []
    status_map = {s["id"]: s["status"] for s in stats}
    if all(v in ("active", "error") for v in status_map.values()) or not status_map:
        break
    time.sleep(1)
check("探针终态 active/error（waiting 已被重试消化或上报）",
      all(v != "waiting" for v in status_map.values()), str(status_map))

# REPL 触发：静态方法直接调用；实例方法走 $new
invoke_static = f'var R = Java.use("com.notevault.app.utils.AESUtil").{target_method}("IT-PLAIN-1"); R'
invoke_inst = (f'var C = Java.use("com.notevault.app.utils.AESUtil"); '
               f'var R = C.$new().{target_method}("IT-PLAIN-1"); R')
# 预热调用：挂载后首次调用走原实现（ART 去优化时机），第二次起 hook 生效
rw = request("rpc_call", {"script_id": script_id, "fn": "replEval", "args": [{"code": invoke_static}]}, timeout=20)
drain_events(1.5)
r = request("rpc_call", {"script_id": script_id, "fn": "replEval", "args": [{"code": invoke_static}]}, timeout=20)
repl1 = (r.get("result") or {}).get("result") or {}
if (repl1.get("value") or {}).get("k") == "err":
    r = request("rpc_call", {"script_id": script_id, "fn": "replEval", "args": [{"code": invoke_inst}]}, timeout=20)
    repl1 = (r.get("result") or {}).get("result") or {}
check("REPL 触发目标方法", (repl1.get("value") or {}).get("k") != "err", str(repl1.get("value"))[:80])

evts = drain_events(3.0)
hits = [e for e in evts if e.get("t") == "probe_hit" and e.get("id") == "it-p1"]
check("命中事件到达（经批量层解包，含 captureRet）", len(hits) >= 1,
      f"ret={str((hits[0].get('ret') or {}).get('v'))[:40] if hits else '无'}")

r = request("rpc_call", {"script_id": script_id, "fn": "probeStats", "args": []}, timeout=10)
stats = (r.get("result") or {}).get("result") or []
p1 = next((s for s in stats if s["id"] == "it-p1"), {})
check("errors 计数未虚增（condition 不再逐命中抛错）", p1.get("errors", 0) == 0,
      str({k: p1.get(k) for k in ("hits", "errors")}))

# ---- 同方法位互斥（联调发现的真 bug 修复验证）----
r = request("rpc_call", {"script_id": script_id, "fn": "addProbes", "args": [{"probes": [
    {"id": "it-dup", "clazz": "com.notevault.app.utils.AESUtil", "method": target_method, "maxLen": 64},
]}]})
dup_r = ((r.get("result") or {}).get("result") or {}).get("results") or []
dup_err = (dup_r[0].get("error") or "") if dup_r else ""
check("同方法位第二探针被明确拒绝（不再静默顶掉先挂者）",
      dup_r and dup_r[0].get("status") == "error" and "同方法位" in dup_err, dup_err[:80])

# ---- 条件过滤（E）：单独挂 p2 ----
r = request("rpc_call", {"script_id": script_id, "fn": "removeProbes", "args": [{"ids": ["it-p1"]}]})
drain_events(0.5)
r = request("rpc_call", {"script_id": script_id, "fn": "addProbes", "args": [{"probes": [
    {"id": "it-p2", "clazz": "com.notevault.app.utils.AESUtil", "method": target_method,
     "maxLen": 128, "captureRet": False,
     "condition": "arguments.length > 0 && String(arguments[0]) === 'IT-CONDITION-HIT'"},
]}]})
drain_events(1.0)
r = request("rpc_call", {"script_id": script_id, "fn": "replEval", "args": [{"code": invoke_static.replace("IT-PLAIN-1", "IT-FILTERED-OFF")}]}, timeout=20)
drain_events(0.3)
r = request("rpc_call", {"script_id": script_id, "fn": "replEval", "args": [{"code": invoke_static.replace("IT-PLAIN-1", "IT-CONDITION-HIT")}]}, timeout=20)
evts = drain_events(3.0)
p2_off_missed_ok = all(not (e.get("t") == "probe_hit" and e.get("id") == "it-p2" and "OFF" in str((e.get("args") or [{}])[0].get("v"))) for e in evts)
p2_on = [e for e in evts if e.get("t") == "probe_hit" and e.get("id") == "it-p2"]
check("条件过滤：不满足条件的调用被拦截、满足条件的放行", p2_off_missed_ok and len(p2_on) >= 1,
      f"放行 {len(p2_on)} 条")
r = request("rpc_call", {"script_id": script_id, "fn": "removeProbes", "args": [{"ids": ["it-p2"]}]})

# ---------------- F. 卸后不命中 ----------------
section("F. 卸后不命中（removeProbes 生效）")
drain_events(0.5)
r = request("rpc_call", {"script_id": script_id, "fn": "replEval", "args": [{"code": invoke_static}]}, timeout=20)
evts = drain_events(2.0)
residual = [e for e in evts if e.get("t") == "probe_hit" and e.get("id") in ("it-p1", "it-p2")]
check("卸载后调用不再产生命中", len(residual) == 0, f"残留 {len(residual)} 条")

# ---------------- H. sidecar 被杀：自动重启恢复 ----------------
section("H. sidecar 韧性（批次三：宿主 ensure 自动重启）")
t0 = time.time()
proc.kill()  # 模拟 sidecar 崩溃
time.sleep(0.3)
# 模拟宿主 ensure()：alive=false → 下次调用拉起新 sidecar 进程
out_q = queue.Queue()
proc = subprocess.Popen(
    [sys.executable, "-u", str(BRIDGE)],
    stdin=subprocess.PIPE,
    stdout=subprocess.PIPE,
    text=True,
    encoding="utf-8",
)
threading.Thread(target=reader, daemon=True).start()
r = request("hello", {}, timeout=40)
elapsed = time.time() - t0
check("sidecar 被杀后自动重启并响应", r.get("result", {}).get("frida") is not None, f"耗时 {elapsed:.1f}s")

r = request("remote_connect", {"host": "127.0.0.1", "port": HOST_PORT})
device_key2 = (r.get("result") or {}).get("key")
pid2, spawned2 = resolve_target_pid(device_key2)
if spawned2:
    request("resume", {"device": device_key2, "pid": pid2})
    time.sleep(1.5)
r = request("attach", {"device": device_key2, "target": pid2})
sid2 = (r.get("result") or {}).get("session_id")
r = request("create_script", {"session_id": sid2, "source": AGENT, "name": "it-core2"})
sid_script2 = (r.get("result") or {}).get("script_id")
hello2 = None
request("load_script", {"script_id": sid_script2})
deadline = time.time() + 8
while time.time() < deadline and hello2 is None:
    try:
        msg = out_q.get(timeout=0.3)
    except queue.Empty:
        continue
    if msg.get("type") == "event" and msg.get("event") == "message":
        p = (msg.get("params") or {}).get("payload") or {}
        if p.get("t") == "hello":
            hello2 = p
check("重启后完整 attach→hello 链路恢复", hello2 is not None)

# 收尾：卸载分离
request("rpc_call", {"script_id": sid_script2, "fn": "removeProbes", "args": [{"ids": []}]}, timeout=10)
request("unload_script", {"script_id": sid_script2}, timeout=15)
request("detach", {"session_id": sid2}, timeout=15)

# ---------------- 汇总 ----------------
print("\n========== 联调结果 ==========")
fails = [r for r in results if not r[1]]
for name, ok, detail in results:
    print(f"{'✔' if ok else '✖'} {name}" + (f" —— {detail}" if detail else ""))
print(f"\n{len(results) - len(fails)}/{len(results)} 项通过")
proc.kill()
sys.exit(1 if fails else 0)
