#!/usr/bin/env python3
"""M2 全链路验收：附加 → 探索器搜索 TextView.setText → 挂探针 → 自动触发 → 时间轴看到命中。"""
import base64
import json
import sys
import time
import urllib.request
from pathlib import Path

import websocket

SHOTS = Path(__file__).resolve().parent.parent / "logs" / "ui_shots"
SHOTS.mkdir(parents=True, exist_ok=True)


def get_ws():
    last = None
    for host in ("127.0.0.1", "[::1]"):
        try:
            data = json.load(urllib.request.urlopen(f"http://{host}:9223/json", timeout=5))
            for t in data:
                if t.get("type") == "page" and "LovelyFrida" in t.get("title", ""):
                    return t["webSocketDebuggerUrl"]
        except Exception as e:  # noqa: BLE001
            last = e
    raise RuntimeError(f"CDP 未就绪: {last}")


def js(expression, timeout=90):
    ws = websocket.create_connection(get_ws(), timeout=timeout)
    try:
        ws.send(json.dumps({"id": 1, "method": "Runtime.evaluate",
                            "params": {"expression": expression, "returnByValue": True,
                                       "awaitPromise": True}}))
        deadline = time.time() + timeout
        while time.time() < deadline:
            m = json.loads(ws.recv())
            if m.get("id") == 1:
                r = m.get("result", {})
                if r.get("exceptionDetails"):
                    raise RuntimeError(json.dumps(r["exceptionDetails"], ensure_ascii=False)[:300])
                return r.get("result", {}).get("value")
        raise TimeoutError(expression[:80])
    finally:
        ws.close()


def wait(expr, timeout_s, desc):
    deadline = time.time() + timeout_s
    last = None
    while time.time() < deadline:
        try:
            last = js(expr, timeout=15)
            if last:
                return last
        except Exception as e:  # noqa: BLE001
            last = f"eval-error: {str(e)[:150]}"
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


def main():
    print("== 1. 设备连接：安装 + 枚举 + 附加 Settings ==")
    js("document.querySelectorAll('.pipeline-node')[1].click()")
    time.sleep(2)
    steps_before = js("""(() => {
      const m = document.body.innerText.match(/共 (\\d+) 步/);
      return m ? Number(m[1]) : 0;
    })()""")
    if steps_before == 0:
        click_button("安装并启动")
    wait(r"""(() => document.querySelectorAll('.install-step').length >= 6 ? true : false)()""", 300, "安装链")
    print("   frida-server 就绪")
    click_button("枚举进程")
    wait(r"""(() => { const m = document.body.innerText.match(/共 (\d+) 条/); return m ? Number(m[1]) >= 1 : false; })()""",
         60, "进程枚举")
    # 优先附加 Settings（有 JVM、UI 会自动画文本，探针必命中）；先系统 tab 再用户 tab
    r = js(r"""(() => {
      const tryClick = () => {
        const rows = [...document.querySelectorAll('tr')];
        const target = rows.find(tr => /settings/i.test(tr.innerText) && tr.querySelector('button'));
        if (target) { const b = [...target.querySelectorAll('button')].find(b => b.innerText.includes('附加')); if (b) { b.click(); return 'attached'; } }
        return null;
      };
      let r = tryClick();
      if (r) return 'attached-user-tab';
      const tabs = [...document.querySelectorAll('.n-tag')].find(t => t.innerText.includes('系统进程'));
      if (tabs) tabs.click();
      return 'switched-to-system';
    })()""")
    time.sleep(1.2)
    r2 = js(r"""(() => {
      const rows = [...document.querySelectorAll('tr')];
      const target = rows.find(tr => /settings/i.test(tr.innerText) && tr.querySelector('button'));
      if (target) { const b = [...target.querySelectorAll('button')].find(b => b.innerText.includes('附加')); if (b) { b.click(); return 'attached-system-tab'; } }
      return 'not-found-in-system';
    })()""")
    print("   附加:", r, r2)
    if 'attached' not in str(r2):
        raise RuntimeError('未找到 Settings 进程行')
    wait(r"""(() => document.body.innerText.includes('hello：frida') ? true : false)()""", 40, "hello 握手")
    print("   hello 握手成功")

    print("== 2. 探针节点：类搜索 TextView → 查看方法 ==")
    js("document.querySelectorAll('.pipeline-node')[4].click()")
    time.sleep(1.5)
    js("""(() => {
      const tabs = [...document.querySelectorAll('.probe-tab')];
      const t = tabs.find(x => x.innerText.includes('探索器'));
      if (t) t.click();
      return 'ok';
    })()""")
    time.sleep(0.8)
    js(r"""(() => {
      const inputs = [...document.querySelectorAll('input')];
      const inp = inputs.find(i => (i.placeholder || '').includes('cipher'));
      if (inp) {
        const setter = Object.getOwnPropertyDescriptor(window.HTMLInputElement.prototype, 'value').set;
        setter.call(inp, 'android.widget.TextView');
        inp.dispatchEvent(new Event('input', { bubbles: true }));
      }
      return 'ok';
    })()""")
    time.sleep(0.4)
    click_button("搜索")
    rows = wait(r"""(() => {
      const btns = [...document.querySelectorAll('button')].filter(b => b.innerText.includes('查看方法'));
      return btns.length > 0 ? btns.length : false;
    })()""", 40, "类搜索结果")
    print(f"   搜到 {rows} 个类")
    js(r"""(() => {
      const b = [...document.querySelectorAll('button')].find(b => b.innerText.includes('查看方法'));
      if (b) b.click();
      return 'ok';
    })()""")
    time.sleep(2)

    print("== 3. 点第一行「挂探针」→ 表单带入 → 挂载 ==")
    js("""(() => {
      const b = [...document.querySelectorAll('button')].find(b => b.innerText.trim() === '挂探针' && b.closest('.explorer-results'));
      if (b) b.click();
    })()""")
    time.sleep(1)
    js("""(() => {
      const t = [...document.querySelectorAll('.probe-tab')].find(x => x.innerText.includes('探针工作台'));
      if (t) t.click();
      return 'ok';
    })()""")
    time.sleep(0.8)
    form_clazz = js("""(() => {
      const inputs = [...document.querySelectorAll('input')];
      const c = inputs.find(i => (i.placeholder || '').includes('完整类名'));
      return c ? c.value : '';
    })()""")
    print(f"   表单已带入: {form_clazz}")
    assert "TextView" in str(form_clazz)
    click_button("挂探针")
    st = wait(r"""(() => {
      const rows = [...document.querySelectorAll('table tbody tr')];
      const p = rows.find(tr => tr.innerText.includes('TextView'));
      if (!p) return false;
      return p.innerText.includes('active') ? 'active' : (p.innerText.includes('waiting') ? 'waiting' : false);
    })()""", 30, "探针 active")
    print("   探针状态:", st)

    print("== 4. 等 TextView.setText 被触发（UI 一直在画字） ==")
    hit = wait(r"""(() => {
      const rows = [...document.querySelectorAll('table tbody tr')];
      const p = rows.find(tr => tr.innerText.includes('TextView'));
      if (!p) return false;
      const m = p.innerText.match(/active\\s*(\\d+)/) || p.innerText.match(/active\\s*\\n?(\\d+)/);
      if (m && Number(m[1]) > 0) return Number(m[1]);
      return false;
    })()""", 60, "探针命中")
    print(f"   命中 {hit} 次（stats 可见）")

    print("== 5. 时间轴视图确认 trace 事件 ==")
    js(r"""(() => {
      window.dispatchEvent(new KeyboardEvent('keydown', { key: '2', bubbles: true }));
      return 'ok';
    })()""")
    time.sleep(1.5)
    n = wait(r"""(() => {
      const m = document.body.innerText.match(/探针触发流 · (\d+) 条/);
      return m ? Number(m[1]) : false;
    })()""", 30, "时间轴事件数")
    print(f"   时间轴事件: {n} 条")
    shot("07-timeline")
    assert n >= 1, "时间轴未收到 trace 事件"
    print("\n✅ M2 全链路验收通过：探索 → 挂探针 → 自动触发 → 时间轴可见")
    return 0


if __name__ == "__main__":
    sys.exit(main())
