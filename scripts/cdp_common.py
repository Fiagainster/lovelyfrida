"""cdp_common.py — CDP 驱动验收脚本的共享层（批次⑮去重）。

cdp_drive*.py 六个脚本此前各自复制 get_ws/js/wait/click_button/shot 一套辅助函数，
复制粘贴式演进导致行为漂移（js 超时 60/90 不一、click_button 有的带 scope）。
统一收敛到这里：
- js()/wait()/click_button()/shot()/fill_by_placeholder()：健壮版「每次求值独立建立
  CDP 连接」（m1 起的纪律——避免长连接状态漂移）；
- CDP 类：持久连接版（cdp_drive.py M0 冒烟专用，长会话截图/多次调用时少建连接）。

前置：应用以 WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS="--remote-debugging-port=9223" 启动。
"""
import base64
import json
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


def get_ws():
    """定位 LovelyFrida 页面目标的 webSocketDebuggerUrl"""
    for t in cdp_http("/json"):
        if t.get("type") == "page" and "LovelyFrida" in t.get("title", ""):
            return t["webSocketDebuggerUrl"]
    raise RuntimeError("未找到 LovelyFrida 页面目标")


def js(expression, timeout=90):
    """单次连接执行一个表达式（awaitPromise，健壮版：求值间互不携带状态）"""
    ws = websocket.create_connection(get_ws(), timeout=timeout)
    try:
        ws.send(json.dumps({"id": 1, "method": "Runtime.evaluate",
                            "params": {"expression": expression, "returnByValue": True,
                                       "awaitPromise": True}}))
        deadline = time.time() + timeout
        while time.time() < deadline:
            raw = ws.recv()
            if not raw:
                continue
            m = json.loads(raw)
            if m.get("id") == 1:
                r = m.get("result", {})
                if r.get("exceptionDetails"):
                    raise RuntimeError(json.dumps(r["exceptionDetails"], ensure_ascii=False)[:300])
                return r.get("result", {}).get("value")
        raise TimeoutError(expression[:80])
    finally:
        ws.close()


def wait(expr, timeout_s, desc):
    """轮询 JS 表达式直到真值（超时抛 TimeoutError，带最后值）"""
    deadline = time.time() + timeout_s
    last = None
    while time.time() < deadline:
        try:
            last = js(expr, timeout=10)
            if last:
                return last
        except Exception as e:  # noqa: BLE001
            last = f"eval-error: {str(e)[:120]}"
        time.sleep(1.5)
    raise TimeoutError(f"{desc}（{str(last)[:200]}）")


def click_button(text, scope=""):
    r = js(f"""(() => {{
      const root = {scope or "document"};
      const b = [...root.querySelectorAll('button')].find(b => b.innerText.includes('{text}'));
      if (b) {{ b.click(); return 'clicked'; }}
      return 'not-found';
    }})()""")
    if r != "clicked":
        raise RuntimeError(f"按钮「{text}」未找到（{r}）")
    return r


def fill_by_placeholder(needle, value):
    """按 placeholder 定位 input 并以原生 setter 赋值（触发 Vue v-model）"""
    return js(rf"""(() => {{
      const inp = [...document.querySelectorAll('input')].find(i => (i.placeholder || '').includes('{needle}'));
      if (!inp) return 'input-not-found';
      const setter = Object.getOwnPropertyDescriptor(window.HTMLInputElement.prototype, 'value').set;
      setter.call(inp, '{value}');
      inp.dispatchEvent(new Event('input', {{ bubbles: true }}));
      return 'filled';
    }})()""")


def shot(name):
    ws = websocket.create_connection(get_ws(), timeout=60)
    try:
        ws.send(json.dumps({"id": 9, "method": "Page.captureScreenshot", "params": {"format": "png"}}))
        deadline = time.time() + 30
        while time.time() < deadline:
            m = json.loads(ws.recv())
            if m.get("id") == 9:
                p = SHOTS / f"{name}.png"
                p.write_bytes(base64.b64decode(m["result"]["data"]))
                print(f"   [截图] {p.name}")
                return
    finally:
        ws.close()


# ---------------- 持久连接版（cdp_drive.py M0 冒烟专用） ----------------

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


def page_ws():
    return get_ws()


def wait_js(cdp, expr, timeout_s, desc):
    """持久连接版轮询（CDP 类配套）"""
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
