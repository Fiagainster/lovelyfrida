#!/usr/bin/env python3
"""M5 全链路验收（U8/U10 路径）：台账登记（high=双证据数据库约束）→ 导出 Markdown → 案卷包。"""
import json
import sys
import time
import urllib.request

import websocket


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


def main():
    ts = time.strftime("%H%M%S")
    answer = f"e2e-answer-{ts}"

    print("== 1. 进入「归档」节点（档案台账） ==")
    js("document.querySelectorAll('.pipeline-node')[7].click()")
    time.sleep(1.5)

    print("== 2. 填发现 + 置信度 high（math+device 双证据勾选） ==")
    assert fill_by_placeholder("问题，如：用户密码是什么", "e2e 问题：验收样例") == "filled"
    assert fill_by_placeholder("分析结论或密码", answer) == "filled"
    # 置信度下拉选 high
    r = js(r"""(() => {
      const sel = [...document.querySelectorAll('.n-base-selection')].find(s => s.innerText.includes('medium') || s.innerText.includes('low') || s.innerText.includes('high'));
      if (!sel) return 'select-not-found';
      sel.dispatchEvent(new MouseEvent('click', { bubbles: true }));
      return 'opened';
    })()""")
    time.sleep(0.6)
    r2 = js(r"""(() => {
      const opt = [...document.querySelectorAll('.n-base-select-option')].find(o => o.innerText.includes('high'));
      if (!opt) return 'option-not-found';
      opt.dispatchEvent(new MouseEvent('click', { bubbles: true }));
      return 'high-selected';
    })()""")
    assert "high" in str(r2), f"置信度下拉未选中 high（{r}/{r2}）"
    # 双证据 tag（math 数学自证 / device 真机复现）
    for tag_text in ("math 数学自证", "device 真机复现"):
        r = js(rf"""(() => {{
          const t = [...document.querySelectorAll('.n-tag')].find(x => x.innerText.includes('{tag_text}'));
          if (!t) return 'tag-not-found';
          t.dispatchEvent(new MouseEvent('click', {{ bubbles: true }}));
          return 'toggled';
        }})()""")
        assert "toggled" in str(r), f"证据 tag「{tag_text}」未找到"
    assert fill_by_placeholder("证据说明", "e2e：数学自证+真机复现（自动验收）") == "filled"
    assert fill_by_placeholder("出处（如：时间轴", "e2e-cdp-m5") == "filled"

    print("== 3. 登记 → 台账出现 high 行 ==")
    click_button("登记")
    row = wait(rf"""(() => {{
      const tr = [...document.querySelectorAll('table tbody tr')].find(x => x.innerText.includes('{answer}'));
      return tr ? tr.innerText.replace(/\s+/g, ' ').slice(0, 120) : false;
    }})()""", 30, "台账新行")
    print("   台账行:", row)
    assert "high" in str(row), "登记行置信度不是 high（双证据约束应放行）"

    print("== 4. 导出 Markdown + 案卷包 ==")
    click_button("导出 Markdown")
    ok1 = wait(r"""(() => document.body.innerText.includes('已导出 →') ? true : false)()""", 30, "md 导出 toast")
    click_button("生成案卷包")
    ok2 = wait(r"""(() => document.body.innerText.includes('案卷包已生成') ? true : false)()""", 60, "案卷包 toast")
    assert ok1 and ok2
    print("\n✅ M5 台账验收通过：high 双证据登记 + md/案卷包导出（U8/U10 路径）")
    return 0


if __name__ == "__main__":
    sys.exit(main())
