#!/usr/bin/env python3
"""M1 验收（健壮版）：每次求值独立建立 CDP 连接，避免长连接状态漂移。"""
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


def js(expression, timeout=60):
    """单次连接执行一个表达式（awaitPromise）"""
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


def wait(expr, timeout_s, desc):
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


def click_button(text):
    r = js(f"""(() => {{
      const b = [...document.querySelectorAll('button')].find(b => b.innerText.includes('{text}'));
      if (b) {{ b.click(); return 'clicked'; }}
      return 'not-found';
    }})()""")
    if r != "clicked":
        raise RuntimeError(f"按钮「{text}」未找到")
    return r


def main():
    print("== 1. Recorder：确认安装链步骤已记录 ==")
    js("document.querySelectorAll('.pipeline-node')[1].click()")
    time.sleep(1.5)
    existing = js(r"""(() => { const m = document.body.innerText.match(/共 (\d+) 步/); return m ? Number(m[1]) : 0; })()""")
    if existing >= 1:
        print(f"   已有 {existing} 步记录，跳过重复安装")
    else:
        click_button("安装并启动")
    rows = wait(r"""(() => { const m = document.body.innerText.match(/共 (\d+) 步/); return m ? Number(m[1]) : false; })()""",
                240, "Recorder 记录（等待安装链完成）")
    print(f"   Recorder 已记录 {rows} 步")
    assert rows >= 1

    print("== 2. 导出 md ==")
    click_button("导出 md")
    time.sleep(1.5)
    toast = js("(() => { const t = document.querySelector('.n-message'); return t ? t.innerText : '(无 toast)'; })()")
    print("   toast:", toast)
    shot("05-recorder")

    print("== 3. 终端抽屉：新建 PTY ==")
    r = js("""(() => {
      const btn = [...document.querySelectorAll('.statusbar__btn')].find(b => b.innerText.includes('终端'));
      if (!btn) return 'no-statusbar-btn';
      btn.click(); return 'opened';
    })()""")
    time.sleep(1)
    click_button("新建终端")
    tab = wait(r"""(() => { const t = document.querySelector('.term-tab'); return t ? t.innerText.replace(/\n/g, ' ') : false; })()""",
               25, "终端会话创建")
    print("   终端:", tab)

    print("== 4. 写入命令 ==")
    term_id = js(r"""(() => { const t = document.querySelector('.term-tab').innerText; return Number(t.match(/term#(\d+)/)[1]); })()""")
    js(f"window.__TAURI_INTERNALS__.invoke('terminal_write', {{ id: {term_id}, data: 'echo LOVELYFRIDA-PTY-OK && id\\r' }})")
    time.sleep(2.5)
    shot("06-terminal")
    print(f"   已写入 term#{term_id}，回显见截图 06")

    print("\n✅ M1 UI 验收执行完成")
    return 0


if __name__ == "__main__":
    sys.exit(main())
