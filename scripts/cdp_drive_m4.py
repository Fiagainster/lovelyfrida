#!/usr/bin/env python3
"""M4 全链路验收（U4+U5 前半）：算法还原（双样本防假命中）→ 内置爆破 HIT → 诊断卡联动。

用可自证的样本：SHA-256('5937'+'somesalt') 与 SHA-256('5938'+'somesalt')。
还原应穷举出「SHA-256 / 明文‖盐 / 单轮 / hex」；爆破填 ?d?d?d?d 应 HIT pwd=5937。
"""
import hashlib
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


def fill_sample(idx, plaintext, salt, target):
    # 三列输入共享 placeholder，按行内顺序定位第 idx 行（0 起）
    return js(rf"""(() => {{
      const rows = [...document.querySelectorAll('table tbody tr')].filter(tr => tr.innerText.includes('样本'));
      if (rows.length <= {idx}) return 'row-missing';
      const inputs = [...rows[{idx}].querySelectorAll('input')];
      if (inputs.length < 3) return 'inputs-missing';
      const setter = Object.getOwnPropertyDescriptor(window.HTMLInputElement.prototype, 'value').set;
      setter.call(inputs[0], '{plaintext}');
      inputs[0].dispatchEvent(new Event('input', {{ bubbles: true }}));
      setter.call(inputs[1], '{salt}');
      inputs[1].dispatchEvent(new Event('input', {{ bubbles: true }}));
      setter.call(inputs[2], '{target}');
      inputs[2].dispatchEvent(new Event('input', {{ bubbles: true }}));
      return 'filled';
    }})()""")


def fill_brute(mask, salt, pwd):
    return js(rf"""(() => {{
      const find = (needle, v) => {{
        const inp = [...document.querySelectorAll('input')].find(i => (i.placeholder || '').includes(needle));
        if (!inp) return 'missing:' + needle;
        const setter = Object.getOwnPropertyDescriptor(window.HTMLInputElement.prototype, 'value').set;
        setter.call(inp, v);
        inp.dispatchEvent(new Event('input', {{ bubbles: true }}));
        return 'filled';
      }};
      return [find('掩码', '{mask}'), find('盐（同还原时）', '{salt}'), find('自测明文', '{pwd}')].join(',');
    }})()""")


def main():
    salt = "somesalt"
    t1 = hashlib.sha256(b"5937" + salt.encode()).hexdigest()
    t2 = hashlib.sha256(b"5938" + salt.encode()).hexdigest()

    print("== 1. 进入「还原」节点 ==")
    js("document.querySelectorAll('.pipeline-node')[6].click()")
    time.sleep(1.5)

    print("== 2. 双样本还原（防假命中） ==")
    assert fill_sample(0, "5937", salt, t1) == "filled"
    assert fill_sample(1, "5938", salt, t2) == "filled", "第二行样本输入未找到"
    click_button("运行还原")
    desc = wait(r"""(() => {
      const el = document.querySelector('.scheme-result .mono');
      return el ? el.innerText : false;
    })()""", 600, "穷举还原出方案")
    print("   方案:", str(desc)[:120])
    assert "SHA-256" in str(desc) or "sha256" in str(desc).lower(), "还原出的不是 SHA-256 族"

    print("== 3. 爆破编排：预估 → 内置爆破 ==")
    assert "filled" in fill_brute("?d?d?d?d", salt, "5937"), "爆破表单填写失败"
    click_button("预估")
    wait(r"""(() => document.body.innerText.includes('总数') ? true : false)()""", 30, "预估三件套")
    click_button("内置爆破")
    hit = wait(r"""(() => {
      const m = document.body.innerText.match(/pwd=([0-9a-zA-Z]+)/);
      return m ? m[1] : false;
    })()""", 120, "HIT 命中")
    print("   爆破命中:", hit)
    assert hit == "5937", f"命中值错误：{hit}"

    print("== 4. 诊断流联动（C-06/C-07 触发器不误报） ==")
    diag = js(r"""(() => {
      const chips = [...document.querySelectorAll('.side-panel .side-hint b')].map(x => x.innerText);
      return chips.filter(x => x.startsWith('C-')).join(',');
    })()""")
    print("   C 组诊断卡:", diag or "（无）")
    assert "C-07" not in str(diag), "自测通过时不应出 C-07 阻断卡"
    print("\n✅ M4 还原+爆破验收通过：双样本定方案，内置爆破 HIT 5937（U4/U5 路径）")
    return 0


if __name__ == "__main__":
    sys.exit(main())
