#!/usr/bin/env python3
"""通过 WebView2 CDP 驱动 LovelyFrida 真实 UI 做按钮级验收。

前置：应用以 WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS="--remote-debugging-port=9223" 启动。
CDP 监听在 [::1]:9223（IPv6）。截图输出到 logs/ui_shots/。
"""
import base64
import json
import sys
import time
import urllib.request
from pathlib import Path

import websocket

SHOTS = Path(__file__).resolve().parent.parent / "logs" / "ui_shots"
SHOTS.mkdir(parents=True, exist_ok=True)


def cdp_http(path):
    last = None
    for host in ("127.0.0.1", "[::1]"):
        try:
            return json.load(urllib.request.urlopen(f"http://{host}:9223{path}", timeout=5))
        except Exception as e:  # noqa: BLE001
            last = e
    raise last


def page_ws():
    for t in cdp_http("/json"):
        if t.get("type") == "page" and "LovelyFrida" in t.get("title", ""):
            return t["webSocketDebuggerUrl"]
    raise RuntimeError("未找到 LovelyFrida 页面目标")


class CDP:
    def __init__(self, url):
        self.ws = websocket.create_connection(url, timeout=60)
        self.n = 0

    def call(self, method, **params):
        self.n += 1
        self.ws.send(json.dumps({"id": self.n, "method": method, "params": params}))
        deadline = time.time() + 120
        while time.time() < deadline:
            msg = json.loads(self.ws.recv())
            if msg.get("id") == self.n:
                if "error" in msg:
                    raise RuntimeError(f"{method}: {msg['error']}")
                return msg.get("result", {})
        raise TimeoutError(method)

    def js(self, expression, await_promise=True):
        r = self.call(
            "Runtime.evaluate",
            expression=expression,
            returnByValue=True,
            awaitPromise=await_promise,
        )
        if r.get("exceptionDetails"):
            raise RuntimeError(json.dumps(r["exceptionDetails"], ensure_ascii=False)[:400])
        return r.get("result", {}).get("value")

    def shot(self, name):
        data = self.call("Page.captureScreenshot", format="png")["data"]
        p = SHOTS / f"{name}.png"
        p.write_bytes(base64.b64decode(data))
        print(f"   [截图] {p.name}")


def wait_js(cdp, expr, timeout_s, desc):
    """轮询 JS 表达式直到真值"""
    deadline = time.time() + timeout_s
    last = None
    while time.time() < deadline:
        try:
            last = cdp.js(expr)
            if last:
                return last
        except Exception as e:  # noqa: BLE001
            last = f"eval-error: {e}"
        time.sleep(1.0)
    raise TimeoutError(f"{desc} 超时（最后值：{str(last)[:200]}）")


def main():
    ws = page_ws()
    cdp = CDP(ws)
    cdp.call("Page.enable")
    print("== 0. 环境 ==")
    is_tauri = cdp.js("('__TAURI_INTERNALS__' in window)")
    print("   Tauri IPC 可用:", is_tauri)
    assert is_tauri, "不在 Tauri 运行时内"

    print("== 1. 切到「设备连接」节点 ==")
    cdp.js("document.querySelectorAll('.pipeline-node')[1].click()")
    time.sleep(2.0)
    # SessionConsole onMounted 已发 frida_server_status；等环境卡出现版本号
    env = wait_js(
        cdp,
        "(() => { const t = document.body.innerText; return t.includes('① 本机客户端') ? (t.match(/17\\.[0-9.]+/) || [''])[0] : false; })()",
        15,
        "frida 环境卡加载",
    )
    print("   本机客户端版本:", env)
    cdp.shot("01-env-card")

    print("== 2. 点击「安装并启动」（幂等安装链，约 20~40s）==")
    cdp.js(
        "[...document.querySelectorAll('button')].find(b => b.innerText.includes('安装并启动')).click()"
    )
    steps = wait_js(
        cdp,
        "(() => { const n = document.querySelectorAll('.install-step').length; return n >= 5 ? n : false; })()",
        90,
        "安装步骤渲染",
    )
    print("   安装步骤渲染:", steps, "项")
    step_text = cdp.js(
        "[...document.querySelectorAll('.install-step')].map(d => d.innerText.replace(/\\n/g, ' | ')).join('\\n')"
    )
    print(step_text)
    has_fail = "✖" in step_text or cdp.js(
        "[...document.querySelectorAll('.install-step .status-light')].some(d => d.className.includes('fail'))"
    )
    if has_fail:
        cdp.shot("02-install-FAIL")
        raise RuntimeError("安装链出现失败步骤")
    cdp.shot("02-install-steps")

    print("== 3. 点击「枚举进程」==")
    cdp.js("[...document.querySelectorAll('button')].find(b => b.innerText.includes('枚举进程')).click()")
    count = wait_js(
        cdp,
        "(() => { const m = document.body.innerText.match(/共 (\\d+) 条/); return m ? Number(m[1]) : false; })()",
        30,
        "进程列表",
    )
    print("   枚举到进程:", count)
    cdp.shot("03-processes")

    print("== 4. 点击第一行「附加」==")
    cdp.js(
        "[...document.querySelectorAll('button')].find(b => b.innerText.includes('附加')).click()"
    )
    hello = wait_js(
        cdp,
        "(() => { const h = document.querySelector('.hello-box'); return h ? h.innerText : false; })()",
        30,
        "hello 握手",
    )
    print("   ", hello)
    phase = cdp.js(
        "(() => { const el = document.querySelector('.phase-node--done:last-of-type, .phase-node--active'); return el ? el.innerText.replace(/\\n/g, ' ') : '?'; })()"
    )
    print("   当前步进:", phase)
    cdp.shot("04-attached")

    print("== 5. 结论 ==")
    ok = cdp.js("document.body.innerText.includes('hello：frida')")
    print("   UI 验收:", "✅ 全部通过" if ok else "❌ 未确认")
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
