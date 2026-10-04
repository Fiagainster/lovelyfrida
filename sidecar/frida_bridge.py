#!/usr/bin/env python3
"""frida_bridge.py — 通道B sidecar：frida-python 官方绑定 ↔ Rust 宿主 的 JSON-RPC 桥。

协议（换行分隔 JSON，stdout 单向输出）：
  宿主 → sidecar : {"id":N,"method":"...","params":{...}}
  sidecar → 宿主 : {"type":"response","id":N,"result":...} | {"type":"response","id":N,"error":"..."}
                   {"type":"event","event":"message|detached|device_lost|spawn_added|...","params":{...}}

设计纪律（文档02核心决策）：结构化数据一律 JSON/base64 回传，宿主从不解析文本输出。
"""
import base64
import json
import queue
import sys
import threading

import frida

# stdio 编码钉死 UTF-8：Windows 管道下 Python 默认随 locale（GBK），进程名/应用名
# 一含非 ASCII 就输出 GBK 字节，宿主按 UTF-8 读行会解码失败。PyInstaller onefile
# 的 bootloader 会清洗 PYTHON* 环境变量（PYTHONUTF8 传不进来），必须源头上钉死。
try:
    sys.stdout.reconfigure(encoding="utf-8", errors="replace")
    sys.stdin.reconfigure(encoding="utf-8", errors="replace")
    sys.stderr.reconfigure(encoding="utf-8", errors="replace")
except Exception:  # noqa: BLE001  老版本 Python 无 reconfigure 时按原样运行
    pass

OUT = queue.Queue()
DEVICE_LOCK = threading.Lock()
DEVICES = {}   # key "host:port" → frida Device
SESSIONS = {}  # session_id(int) → {"device": key, "session": Session}
SCRIPTS = {}   # script_id(int) → {"session_id": int, "script": Script}
SEQ = {"n": 0}
SEQ_LOCK = threading.Lock()


def _next_id() -> int:
    with SEQ_LOCK:
        SEQ["n"] += 1
        return SEQ["n"]


def emit(obj) -> None:
    OUT.put(obj)


def reply(req_id, result=None, error=None) -> None:
    emit({"type": "response", "id": req_id, "result": result, "error": error})


def notify(event: str, params) -> None:
    emit({"type": "event", "event": event, "params": params})


def _data_to_b64(data):
    if data is None:
        return None
    return base64.b64encode(data).decode("ascii")


# ---------------- frida 回调 → 事件 ----------------

def _make_on_message(script_id):
    def on_message(message, data):
        notify("message", {
            "script_id": script_id,
            "kind": message.get("type"),          # send | error | log
            "payload": message.get("payload"),
            "description": message.get("description"),
            "stack": message.get("stack"),
            "data_b64": _data_to_b64(data),
        })
    return on_message


def _make_on_detached(session_id):
    def on_detached(reason, crash):
        notify("detached", {
            "session_id": session_id,
            "reason": str(reason),
            "crash": None if crash is None else {
                "process_name": getattr(crash, "process_name", None),
                "report": getattr(crash, "report", None),
                "summary": getattr(crash, "summary", None),
            },
        })
        with DEVICE_LOCK:
            SESSIONS.pop(session_id, None)
    return on_detached


# ---------------- 方法实现 ----------------

def m_hello(params):
    return {
        "frida": frida.__version__,
        "python": sys.version.split()[0],
        "bridge": "1.0",
    }


def m_remote_connect(params):
    host = params.get("host", "127.0.0.1")
    port = int(params.get("port", 27042))
    key = f"{host}:{port}"
    with DEVICE_LOCK:
        dev = DEVICES.get(key)
        if dev is None:
            mgr = frida.get_device_manager()
            dev = mgr.add_remote_device(key)
            DEVICES[key] = dev
            # 设备失联事件（设备掉线/重启）
            dev.on("lost", lambda: notify("device_lost", {"device": key}))
    return {"key": key, "id": dev.id, "name": dev.name, "type": dev.type}


def m_enumerate_processes(params):
    key = params.get("device")
    with DEVICE_LOCK:
        dev = DEVICES[key]
        procs = dev.enumerate_processes()
    return [{"pid": p.pid, "name": p.name} for p in procs]


def m_enumerate_applications(params):
    key = params.get("device")
    with DEVICE_LOCK:
        dev = DEVICES[key]
        apps = dev.enumerate_applications()
    return [
        {"identifier": a.identifier, "name": a.name, "pid": a.pid}
        for a in apps
    ]


def m_spawn(params):
    key = params.get("device")
    program = params.get("program")
    with DEVICE_LOCK:
        dev = DEVICES[key]
    pid = dev.spawn(program)
    return {"pid": pid}


def m_resume(params):
    key = params.get("device")
    pid = int(params["pid"])
    with DEVICE_LOCK:
        dev = DEVICES[key]
    dev.resume(pid)
    return {"ok": True}


def m_kill(params):
    key = params.get("device")
    pid = int(params["pid"])
    with DEVICE_LOCK:
        dev = DEVICES[key]
    dev.kill(pid)
    return {"ok": True}


def m_attach(params):
    key = params.get("device")
    target = params.get("target")  # pid(int) 或进程名(str)
    with DEVICE_LOCK:
        dev = DEVICES[key]
    session = dev.attach(target)
    session_id = _next_id()
    with DEVICE_LOCK:
        SESSIONS[session_id] = {"device": key, "session": session}
    session.on("detached", _make_on_detached(session_id))
    return {"session_id": session_id}


def m_enable_child_gating(params):
    sid = int(params["session_id"])
    with DEVICE_LOCK:
        SESSIONS[sid]["session"].enable_child_gating()
    return {"ok": True}


def m_create_script(params):
    sid = int(params["session_id"])
    source = params["source"]
    name = params.get("name", "agent")
    with DEVICE_LOCK:
        session = SESSIONS[sid]["session"]
    script = session.create_script(source=source, name=name)
    script_id = _next_id()
    with DEVICE_LOCK:
        SCRIPTS[script_id] = {"session_id": sid, "script": script}
    script.on("message", _make_on_message(script_id))
    return {"script_id": script_id}


def m_load_script(params):
    script_id = int(params["script_id"])
    with DEVICE_LOCK:
        SCRIPTS[script_id]["script"].load()
    return {"ok": True}


def m_unload_script(params):
    script_id = int(params["script_id"])
    with DEVICE_LOCK:
        entry = SCRIPTS.pop(script_id, None)
    if entry:
        entry["script"].unload()
    return {"ok": True}


def m_post(params):
    script_id = int(params["script_id"])
    message = params["message"]
    data_b64 = params.get("data_b64")
    data = base64.b64decode(data_b64) if data_b64 else None
    with DEVICE_LOCK:
        SCRIPTS[script_id]["script"].post(message, data=data)
    return {"ok": True}


def m_rpc_call(params):
    script_id = int(params["script_id"])
    fn = params["fn"]
    args = params.get("args", [])
    with DEVICE_LOCK:
        script = SCRIPTS[script_id]["script"]
    exports = getattr(script, "exports_sync", None) or script.exports
    func = getattr(exports, fn)
    result = func(*args)
    return {"result": result}


def m_detach(params):
    sid = int(params["session_id"])
    with DEVICE_LOCK:
        entry = SESSIONS.pop(sid, None)
    if entry:
        entry["session"].detach()
    return {"ok": True}


SPAWN_GATING_BOUND = set()  # 已注册 spawn-added 回调的 device key（重复调用会叠加 handler）


def m_enable_spawn_gating(params):
    key = params.get("device")
    with DEVICE_LOCK:
        dev = DEVICES[key]
    dev.enable_spawn_gating()
    if key not in SPAWN_GATING_BOUND:
        SPAWN_GATING_BOUND.add(key)
        dev.on("spawn-added", lambda spawn: notify("spawn_added", {
            "identifier": getattr(spawn, "identifier", None),
            "pid": getattr(spawn, "pid", None),
        }))
    return {"ok": True}


METHODS = {
    "hello": m_hello,
    "remote_connect": m_remote_connect,
    "enumerate_processes": m_enumerate_processes,
    "enumerate_applications": m_enumerate_applications,
    "spawn": m_spawn,
    "resume": m_resume,
    "kill": m_kill,
    "attach": m_attach,
    "enable_child_gating": m_enable_child_gating,
    "create_script": m_create_script,
    "load_script": m_load_script,
    "unload_script": m_unload_script,
    "post": m_post,
    "rpc_call": m_rpc_call,
    "detach": m_detach,
    "enable_spawn_gating": m_enable_spawn_gating,
}


def handle(line: str) -> None:
    try:
        req = json.loads(line)
    except Exception as e:  # noqa: BLE001
        reply(-1, error=f"bad request json: {e}")
        return
    req_id = req.get("id", -1)
    method = req.get("method", "")
    params = req.get("params") or {}
    handler = METHODS.get(method)
    if handler is None:
        reply(req_id, error=f"unknown method: {method}")
        return
    try:
        reply(req_id, result=handler(params))
    except Exception as e:  # noqa: BLE001
        reply(req_id, error=f"{type(e).__name__}: {e}")


def writer_loop() -> None:
    # 单写线程纪律不变；攒走「已就绪」的行合并为一次 write+flush：
    # 高频事件下把 flush 系统调用从每条一次摊薄到每批一次，且不给响应增加额外延迟
    while True:
        obj = OUT.get()
        batch = [obj]
        while len(batch) < 64:
            try:
                batch.append(OUT.get_nowait())
            except queue.Empty:
                break
        try:
            sys.stdout.write(
                "".join(json.dumps(o, ensure_ascii=False, default=str) + "\n" for o in batch)
            )
            sys.stdout.flush()
        except Exception:  # noqa: BLE001
            break  # stdout 已断（宿主退出）：写线程退出，进程随 stdin EOF 收尾


def main() -> None:
    threading.Thread(target=writer_loop, daemon=True).start()
    notify("ready", {"frida": frida.__version__})
    for line in sys.stdin:
        line = line.strip()
        if not line:
            continue
        # 每个请求独立线程处理，保持 stdin 循环响应（frida 调用可能阻塞）
        threading.Thread(target=handle, args=(line,), daemon=True).start()
    # stdin EOF → 优雅清理
    with DEVICE_LOCK:
        scripts = list(SCRIPTS.values())
        sessions = list(SESSIONS.values())
    for entry in scripts:
        try:
            entry["script"].unload()
        except Exception:  # noqa: BLE001
            pass
    for entry in sessions:
        try:
            entry["session"].detach()
        except Exception:  # noqa: BLE001
            pass


if __name__ == "__main__":
    main()
