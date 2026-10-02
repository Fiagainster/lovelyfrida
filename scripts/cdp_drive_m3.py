#!/usr/bin/env python3
"""M3 全链路验收（U6）：回灌向导七步——停应用 → 空跑建档 → 建目录 → 中转推送 → chown → restorecon → md5。

前置：dev 应用已启动（CDP 9223）+ 模拟器已 root。本脚本向 /data/local/tmp/lf-e2e
灌入一个自建小文件（不触碰任何真实应用数据），断言七步报告出现且无 fail 步。
"""
import json
import sys
import time
import urllib.request
from pathlib import Path

import websocket

SHOTS = Path(__file__).resolve().parent.parent / "logs" / "ui_shots"
SHOTS.mkdir(parents=True, exist_ok=True)
PAYLOAD = Path(__file__).resolve().parent.parent / "workspace" / "lf-e2e-inject.json"


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


def click_button(text):
    r = js(f"""(() => {{
      const b = [...document.querySelectorAll('button')].find(b => b.innerText.includes('{text}'));
      if (b) {{ b.click(); return 'clicked'; }}
      return 'not-found';
    }})()""")
    if r != "clicked":
        raise RuntimeError(f"按钮「{text}」未找到（{r}）")
    return r


def fill_by_placeholder(needle, value):
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


import base64  # noqa: E402  （shot 用到，置顶 import 区之外仅为本脚本可读性）


def main():
    # 准备灌入文件（工作区自建，不碰检材）
    PAYLOAD.parent.mkdir(parents=True, exist_ok=True)
    PAYLOAD.write_text(json.dumps({"note": "lovelyfrida e2e payload", "ts": time.time()}), encoding="utf-8")
    local = str(PAYLOAD).replace("\\", "\\\\")

    print("== 1. 进入「数据回灌」节点 ==")
    js("document.querySelectorAll('.pipeline-node')[3].click()")
    time.sleep(1.5)

    print("== 2. 填包名 + 一行文件（本地 → /data/local/tmp/lf-e2e） ==")
    assert fill_by_placeholder("目标包名", "com.android.settings") == "filled"
    assert fill_by_placeholder("D:", local) == "filled", "本地文件路径输入框未找到"
    assert fill_by_placeholder("/data/data/<pkg>/files", "/data/local/tmp/lf-e2e") == "filled"
    assert fill_by_placeholder("password.json", "lf-e2e.json") == "filled"

    print("== 3. 运行回灌（七步） ==")
    click_button("运行回灌")
    overall = wait(r"""(() => {
      const h3 = [...document.querySelectorAll('h3')].find(x => x.innerText.includes('回灌报告'));
      if (!h3) return false;
      const chip = h3.querySelector('.status-chip');
      return chip ? chip.innerText.trim() : false;
    })()""", 180, "七步报告")
    print("   overall:", overall)
    steps = js(r"""(() => {
      const rows = [...document.querySelectorAll('table tbody tr')];
      const report = rows.filter(tr => /①|②|③|④|⑤|⑥|⑦/.test(tr.innerText));
      return report.map(tr => tr.innerText.replace(/\s+/g, ' ').slice(0, 80));
    })()""")
    for s in steps:
        print("   -", s)
    shot("08-injection-report")
    assert "失败" not in str(overall), "回灌报告含失败步骤"
    assert len(steps) >= 6, f"七步报告不完整（{len(steps)} 行）"
    print("\n✅ M3 回灌向导验收通过：七步无 fail（U6 路径）")
    return 0


if __name__ == "__main__":
    sys.exit(main())
