#!/usr/bin/env python3
"""it_sidecar_lifecycle.py — 通道B 请求生命周期冒烟（批次⑩，无需真机/设备）。

进程内驱动 frida_bridge：把 OP_TIMEOUT_S 调小、注入假 method/假设备表，
验证 ① 快路径 ② 结构化超时 + op_abandoned + op_late 迟到完成 ③ 创建型副作用回滚
④ device_lost 登记清理与 detached 代发。CI/本地均可跑（只依赖 pip install frida）。

用法：python scripts/it_sidecar_lifecycle.py   （全过退出码 0，否则 1）
"""
import sys
import threading
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent / "sidecar"))

import frida_bridge as fb  # noqa: E402

PASS = []
FAIL = []


def check(name, cond, detail=""):
    (PASS if cond else FAIL).append(name)
    print(f"  {'✓' if cond else '✖'} {name}{'' if cond else f' —— {detail}'}")


class FakeSession:
    def __init__(self):
        self.detached = False

    def detach(self):
        self.detached = True


class FakeScript:
    def __init__(self):
        self.unloaded = False

    def unload(self):
        self.unloaded = True


def drain_out():
    items = []
    while True:
        try:
            items.append(fb.OUT.get_nowait())
        except Exception:  # noqa: BLE001
            return items


def main() -> int:
    fb.OP_TIMEOUT_S = 1.0  # 冒烟用小超时（handle 运行时读取，可注入）

    # ① 快路径：hello 正常响应
    fb.handle('{"id":1,"method":"hello","params":{}}')
    msgs = drain_out()
    hello = next((m for m in msgs if m.get("type") == "response" and m.get("id") == 1), None)
    check("hello 快路径正常响应", hello is not None and hello.get("error") is None
          and hello.get("result", {}).get("bridge") == "1.1", str(hello))

    # ② 未知方法
    fb.handle('{"id":2,"method":"nope","params":{}}')
    msgs = drain_out()
    check("未知方法返回 error", any(m.get("id") == 2 and m.get("error") for m in msgs))

    # ③ 超时 → 弃管 → 迟到完成补发 op_late（注入慢 method）
    def slow(params):
        time.sleep(2.0)
        return {"done": True}

    fb.METHODS["slow_test"] = slow
    fb.handle('{"id":3,"method":"slow_test","params":{}}')
    msgs = drain_out()
    resp = next((m for m in msgs if m.get("type") == "response" and m.get("id") == 3), None)
    check("超时收到结构化 timeout 错误（而非静默）",
          resp is not None and "timeout: slow_test" in str(resp.get("error")), str(resp))
    check("超时伴随 op_abandoned 事件",
          any(m.get("event") == "op_abandoned" and m["params"].get("method") == "slow_test" for m in msgs if m.get("type") == "event"))
    time.sleep(1.6)  # 等底层调用（2s）完成
    msgs = drain_out()
    late = next((m for m in msgs if m.get("type") == "event" and m.get("event") == "op_late"), None)
    check("弃管请求迟到完成补发 op_late（ok=True 带结果）",
          late is not None and late["params"].get("ok") is True and late["params"].get("result") == {"done": True}, str(late))

    # ④ 创建型副作用回滚（直接验证回滚逻辑）
    fb.SESSIONS[77] = {"device": "127.0.0.1:1", "session": (s := FakeSession())}
    fb._rollback_abandoned("attach", {}, {"session_id": 77})
    check("attach 迟到成功 → 会话回滚 detach + 登记清除", s.detached and 77 not in fb.SESSIONS)

    fb.SCRIPTS[88] = {"session_id": 77, "script": (sc := FakeScript())}
    fb._rollback_abandoned("create_script", {}, {"script_id": 88})
    check("create_script 迟到成功 → 脚本回滚 unload + 登记清除", sc.unloaded and 88 not in fb.SCRIPTS)

    fb.SCRIPTS[99] = {"session_id": 77, "script": (sc2 := FakeScript())}
    fb._rollback_abandoned("load_script", {"script_id": 99}, None)
    check("load_script 迟到成功 → 脚本回滚 unload + 登记清除", sc2.unloaded and 99 not in fb.SCRIPTS)
    drain_out()

    # ⑤ device_lost：登记清理 + detached 代发
    lost_session = FakeSession()
    fb.DEVICES["127.0.0.1:9"] = object()
    fb.SESSIONS[55] = {"device": "127.0.0.1:9", "session": lost_session}
    fb.SCRIPTS[66] = {"session_id": 55, "script": FakeScript()}
    on_lost = fb._make_on_lost("127.0.0.1:9")
    on_lost()
    msgs = drain_out()
    check("device_lost 后 DEVICES/SESSIONS/SCRIPTS 全清",
          "127.0.0.1:9" not in fb.DEVICES and 55 not in fb.SESSIONS and 66 not in fb.SCRIPTS)
    check("device_lost 代发 detached（宿主状态灯收敛）",
          any(m.get("event") == "detached" and m["params"].get("session_id") == 55
              and m["params"].get("reason") == "device_lost" for m in msgs if m.get("type") == "event"), str(msgs))

    print(f"\n结果：{len(PASS)} 过 / {len(FAIL)} 败")
    return 1 if FAIL else 0


if __name__ == "__main__":
    sys.exit(main())
