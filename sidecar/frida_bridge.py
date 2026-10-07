#!/usr/bin/env python3
"""frida_bridge.py — 通道B sidecar：frida-python 官方绑定 ↔ Rust 宿主 的 JSON-RPC 桥。

协议（换行分隔 JSON，stdout 单向输出）：
  宿主 → sidecar : {"id":N,"method":"...","params":{...}}
  sidecar → 宿主 : {"type":"response","id":N,"result":...} | {"type":"response","id":N,"error":"..."}
                   {"type":"event","event":"message|detached|device_lost|spawn_added|...","params":{...}}

请求生命周期（bridge 1.1，批次⑩）：frida 调用跑在有限工作池（8 线程）里，每个请求
25s 结构化超时（早于宿主 30s，宿主收到的是明确的 timeout 错误而非裸超时）。Python
线程不可强杀：超时的底层调用可能仍在进行——按「弃管」处理：登记进 ABANDONED，
若最终完成则补发 op_late 事件，并对有状态副作用回滚（spawn→kill 挂起进程、
attach→detach、create_script/load_script→unload），保证宿主已放弃的请求不留孤儿状态。

设计纪律（文档02核心决策）：结构化数据一律 JSON/base64 回传，宿主从不解析文本输出。
"""
import base64
import concurrent.futures
import json
import queue
import sys
import threading
import time

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

# 请求生命周期（批次⑩）：frida 调用收敛进有限工作池——此前 thread-per-request 在
# frida-server 半死时线程无上限累积；工作池满后请求排队，由 25s 超时逐个给出结构化错误。
OP_TIMEOUT_S = 25.0
POOL = concurrent.futures.ThreadPoolExecutor(max_workers=8, thread_name_prefix="op")
ABANDONED_LOCK = threading.Lock()
ABANDONED = {}  # req_id → {"method": str, "params": dict, "future": Future}
# remote_connect 专用：add_remote_device 是慢调用，不能占 DEVICE_LOCK（否则阻塞全部方法），
# 但又要防并发双连——双检收敛在这把独立的锁上。
CONNECT_LOCK = threading.Lock()


def _next_id() -> int:
    with SEQ_LOCK:
        SEQ["n"] += 1
        return SEQ["n"]


# 桥协议版本（批次⑪④）：ready 事件与 m_hello 都上报，宿主在入口处对账。
# 1.1 = 25s 结构化超时 / 工作池 / 弃管回滚 / op_late 事件
BRIDGE_VERSION = "1.1"


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
        "bridge": BRIDGE_VERSION,
    }


def _make_on_lost(key):
    def on_lost():
        notify("device_lost", {"device": key})
        # 状态收敛（批次⑩）：此前失联设备的 Device 对象永留 DEVICES，其下的
        # SESSIONS/SCRIPTS 也不清——后续调用全靠 frida 抛错逐个失败。这里把登记
        # 全部清除并代发 detached（会话已随设备死亡，宿主/UI 状态灯据此收敛）。
        with DEVICE_LOCK:
            DEVICES.pop(key, None)
            dead_sessions = [sid for sid, e in SESSIONS.items() if e["device"] == key]
            for sid in dead_sessions:
                SESSIONS.pop(sid, None)
            dead_scripts = [scid for scid, e in SCRIPTS.items() if e["session_id"] in dead_sessions]
            for scid in dead_scripts:
                SCRIPTS.pop(scid, None)
        for sid in dead_sessions:
            notify("detached", {"session_id": sid, "reason": "device_lost", "crash": None})
    return on_lost


def m_remote_connect(params):
    host = params.get("host", "127.0.0.1")
    port = int(params.get("port", 27042))
    key = f"{host}:{port}"
    with DEVICE_LOCK:
        dev = DEVICES.get(key)
    if dev is not None:
        return {"key": key, "id": dev.id, "name": dev.name, "type": dev.type}
    # add_remote_device 是慢调用（对 frida-server 做握手，半死时能挂死）：
    # 不能占 DEVICE_LOCK；去重用 CONNECT_LOCK 双检。
    with CONNECT_LOCK:
        with DEVICE_LOCK:
            dev = DEVICES.get(key)
        if dev is None:
            mgr = frida.get_device_manager()
            dev = mgr.add_remote_device(key)
            with DEVICE_LOCK:
                DEVICES[key] = dev
            # 设备失联事件（设备掉线/重启）
            dev.on("lost", _make_on_lost(key))
    return {"key": key, "id": dev.id, "name": dev.name, "type": dev.type}


def m_enumerate_processes(params):
    key = params.get("device")
    with DEVICE_LOCK:
        dev = DEVICES[key]
    # 枚举可达数秒，必须在 DEVICE_LOCK 外执行（此前占锁期间 attach/rpc_call 全部排队）
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
        entry = SESSIONS[sid]
    entry["session"].enable_child_gating()
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
        entry = SCRIPTS[script_id]
    # load 编译并运行 agent（可达数秒），必须锁外执行（此前占锁阻塞全部方法）
    entry["script"].load()
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
        entry = SCRIPTS[script_id]
    entry["script"].post(message, data=data)
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


def _rollback_abandoned(method, params, result):
    """弃管请求最终成功时的状态回滚：宿主已收到 timeout 错误，这些副作用没有主。
    回滚是尽力而为（任何异常吞掉——op_late 已留痕）；只回滚「创建型」副作用，
    kill/post/rpc_call 等无法撤销的只补发事件不回滚。"""
    try:
        if method == "spawn":
            # 挂起态进程必须清掉，否则留下一个永远不运行的僵尸进程占着 pid
            key = params.get("device")
            pid = (result or {}).get("pid")
            if key and pid:
                with DEVICE_LOCK:
                    dev = DEVICES.get(key)
                if dev:
                    dev.kill(pid)
        elif method == "attach":
            sid = (result or {}).get("session_id")
            if sid is not None:
                with DEVICE_LOCK:
                    entry = SESSIONS.pop(sid, None)
                if entry:
                    entry["session"].detach()
        elif method == "create_script":
            scid = (result or {}).get("script_id")
            if scid is not None:
                with DEVICE_LOCK:
                    entry = SCRIPTS.pop(scid, None)
                if entry:
                    entry["script"].unload()
        elif method == "load_script":
            scid = params.get("script_id")
            if scid is not None:
                with DEVICE_LOCK:
                    entry = SCRIPTS.pop(scid, None)
                if entry:
                    entry["script"].unload()
    except Exception:  # noqa: BLE001
        pass


def _on_abandoned_done(req_id, method, params, future):
    # 登记已被派发方移除 = 请求其实按时完成（竞态兜底），无事可做
    with ABANDONED_LOCK:
        if ABANDONED.pop(req_id, None) is None:
            return
    result, err = None, None
    try:
        result = future.result()
    except Exception as e:  # noqa: BLE001
        err = f"{type(e).__name__}: {e}"
    notify("op_late", {
        "req_id": req_id,
        "method": method,
        "ok": err is None,
        "result": result,
        "error": err,
    })
    if err is None:
        _rollback_abandoned(method, params, result)


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
    # 工作池 + 结构化超时（批次⑩）：宿主 30s 前收到明确的 timeout 错误；底层调用
    # 若最终完成，由 _on_abandoned_done 补发 op_late 并回滚创建型副作用。
    future = POOL.submit(handler, params)
    try:
        reply(req_id, result=future.result(timeout=OP_TIMEOUT_S))
        return
    except concurrent.futures.TimeoutError:
        pass
    except Exception as e:  # noqa: BLE001
        reply(req_id, error=f"{type(e).__name__}: {e}")
        return
    with ABANDONED_LOCK:
        ABANDONED[req_id] = {"method": method, "params": params, "future": future}
    future.add_done_callback(lambda f, rid=req_id, m=method, p=params: _on_abandoned_done(rid, m, p, f))
    reply(req_id, error=f"timeout: {method}（>{OP_TIMEOUT_S:.0f}s；底层调用仍在进行，完成后将补发 op_late 并回滚创建型副作用）")
    notify("op_abandoned", {"req_id": req_id, "method": method})


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


def _cleanup() -> None:
    # stdin EOF → 优雅清理（尽力而为；脚本卸载/会话分离失败不再兜底范围内）
    with DEVICE_LOCK:
        scripts = list(SCRIPTS.values())
        sessions = list(SESSIONS.values())
        SCRIPTS.clear()
        SESSIONS.clear()
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


def main() -> None:
    threading.Thread(target=writer_loop, daemon=True).start()
    notify("ready", {"frida": frida.__version__, "bridge": BRIDGE_VERSION})
    try:
        for line in sys.stdin:
            line = line.strip()
            if not line:
                continue
            # 每个请求独立派发线程（future.result(25s) 保证其 ≤ 超时即退出），
            # frida 调用本体在 8 线程工作池里跑，保持 stdin 循环响应
            threading.Thread(target=handle, args=(line,), daemon=True).start()
    finally:
        # stdin EOF（宿主正常退出）或读循环异常（IO 错误）都要走清理：
        # 此前异常路径会跳过 unload/detach 直接崩（后果有界但语义不对）
        _cleanup()


if __name__ == "__main__":
    main()
