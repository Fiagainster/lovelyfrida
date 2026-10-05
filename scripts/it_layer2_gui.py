#!/usr/bin/env python3
"""批次三~六联调驱动（层2 GUI）：通过 CDP 驱动真实 UI 走完整业务链。

前置：
  - 设备已连接（本机 MuMu adb 127.0.0.1:5555，config.toml [doctor] emulator_ports 已含）
  - 应用以 WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS="--remote-debugging-port=9223" 启动（CDP 在 [::1]:9223）

链路：体检 → 连接 → 装载（附加 notevault）→ 探针命中 → 时间轴 → sidecar 击杀恢复 →
      U4 真案还原（Wei123123）→ 爆破 → 台账。
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

results: list[tuple[str, bool, str]] = []


def check(name: str, ok: bool, detail: str = "") -> None:
    results.append((name, ok, detail))
    print(f"  {'✔' if ok else '✖'} {name}" + (f" —— {detail}" if detail else ""))


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
    # CDP 偶发 error 帧（页面瞬时刷新/上下文重建）：跳过事件帧 + 重连重试 3 次
    for _attempt in range(3):
        ws = websocket.create_connection(get_ws(), timeout=timeout)
        try:
            ws.send(json.dumps({"id": 1, "method": "Runtime.evaluate",
                                "params": {"expression": expression, "returnByValue": True,
                                           "awaitPromise": True}}))
            deadline = time.time() + timeout
            retry = False
            while time.time() < deadline:
                m = json.loads(ws.recv())
                if m.get("id") != 1:
                    continue  # 事件帧（Log.entry / console API）跳过
                if "error" in m:
                    retry = True
                    break
                r = m.get("result", {})
                if r.get("exceptionDetails"):
                    raise RuntimeError(json.dumps(r["exceptionDetails"], ensure_ascii=False)[:300])
                return r.get("result", {}).get("value")
            if retry:
                time.sleep(0.6)
                continue
            raise TimeoutError(expression[:80])
        finally:
            ws.close()
    raise TimeoutError(expression[:80])


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


def fill_input(placeholder_part, value):
    # value 经 JSON 编码注入：任意内容（含引号/换行）都是合法 JS 字符串字面量
    v = json.dumps(value, ensure_ascii=False)
    return js(f"""(() => {{
      const inp = [...document.querySelectorAll('input')].find(i => (i.placeholder || '').includes({json.dumps(placeholder_part)}));
      if (!inp) return 'missing';
      const setter = Object.getOwnPropertyDescriptor(window.HTMLInputElement.prototype, 'value').set;
      setter.call(inp, {v});
      inp.dispatchEvent(new Event('input', {{ bubbles: true }}));
      return 'filled';
    }})()""")


def fill_sample(idx, plaintext, salt, target):
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


def node(idx):
    js(f"document.querySelectorAll('.pipeline-node')[{idx}].click()")
    time.sleep(1.5)


def main():
    print("== 1. 体检（快速）===")
    node(0)
    click_button("快速体检")
    overall = wait(r"""(() => {{
      const el = document.querySelector('.view-head__sub');
      return document.body.innerText.includes('体检结论') ? document.body.innerText.split('体检结论：')[1].split('\\n')[0] : false;
    }})()""", 180, "体检完成")
    doctor = js(r"""(() => {
      const fails = document.querySelectorAll('.status-chip--fail').length;
      const overall = (document.body.innerText.split('体检结论：')[1] || '').split('\n')[0];
      const dur = (document.body.innerText.match(/([\d.]+)s\s*\n?\s*总耗时/) || [])[1] || '?';
      return JSON.stringify({ fails: fails, overall: overall, dur: dur });
    })()""")
    import json as _json
    d = _json.loads(doctor) if isinstance(doctor, str) and doctor.startswith('{') else {"fails": 99, "overall": str(doctor), "dur": "?"}
    check("体检完成且 0 失败（并行化耗时可见）", d.get("fails") == 0 and d.get("overall") in ("pass", "warn"),
          f"overall={d.get('overall')} 失败项={d.get('fails')} 总耗时={d.get('dur')}s")
    c1btn = str(js("""(() => [...document.querySelectorAll('button')].filter(b => b.innerText.includes('下载 frida-server')).length)()"""))
    check("C1 下载按钮不出现（CHK-08 一致 → 无需下载）", c1btn == "0", c1btn)
    shot("it_01_doctor")

    print("== 2. 连接节点：设备在列 ==")
    node(1)
    devices = wait(r"""(() => document.body.innerText.includes('127.0.0.1:5555') ? '5555-in-list' : false)()""",
                   30, "设备列表")
    check("设备 127.0.0.1:5555 出现在连接面板", devices == "5555-in-list")
    shot("it_02_connect")

    print("== 3. 装载节点：安装链（frida-server 未运行则走 GUI 安装链）+ 附加 notevault ==")
    node(2)
    listening = js("""(() => document.body.innerText.includes('实测监听中') ? 'yes' : 'no')())""")
    if listening != "yes":
        print("   frida-server 未监听 → 点「安装并启动（推送匹配版）」")
        click_button("安装并启动")
        wait(r"""(() => document.querySelectorAll('.install-step').length >= 6 ? true : false)()""", 300, "安装链 6 步")
        check("安装链 6 步全走完（推送/chmod/启动/实测监听）", True)
        js("""(() => { const b = [...document.querySelectorAll('button')].find(x => x.innerText.trim() === '刷新'); if (b) b.click(); return 'ok'; })()""")
    wait(r"""(() => document.body.innerText.includes('实测监听中') ? 'yes' : false)()""", 30, "serverStatus 刷新出实测监听")
    check("frida-server 实测监听（S-05 假绿灯防护后）",
          js("""(() => document.body.innerText.includes('实测监听中') ? 'yes' : 'no')()""") == "yes")
    click_button("枚举进程")
    wait(r"""(() => { const m = document.body.innerText.match(/共 (\d+) 条/); return m ? Number(m[1]) >= 1 : false; })()""",
         90, "进程枚举")
    r = js(r"""(() => {
      const rows = [...document.querySelectorAll('tr')];
      const target = rows.find(tr => /notevault/i.test(tr.innerText) && tr.querySelector('button'));
      if (target) { const b = [...target.querySelectorAll('button')].find(b => b.innerText.includes('附加')); if (b) { b.click(); return 'attached'; } }
      return 'not-found';
    })()""")
    check("找到 notevault 进程行并点附加", r == "attached", str(r))
    wait(r"""(() => document.body.innerText.includes('hello：frida') || document.body.innerText.includes('hello 握手成功') ? true : false)()""",
         60, "hello 握手")
    check("hello 握手成功（会话 Running）", True)
    shot("it_03_attach")

    print("== 4. 探针节点：挂 hashPassword + REPL 触发 ==")
    node(4)
    form_ok = wait(r"""(() => document.body.innerText.includes('挂新探针') ? 'yes' : false)()""", 20, "探针表单")
    check("探针表单出现（会话已就绪）", form_ok == "yes")
    check("填类名", fill_input("完整类名", "com.notevault.app.utils.AESUtil") == "filled")
    check("填方法名", fill_input("方法名", "hashPassword") == "filled")
    click_button("挂探针")
    import time as _t
    _t.sleep(6)
    js("""(() => { const b = [...document.querySelectorAll('button')].find(x => x.innerText.includes('刷新状态')); b && b.click(); return 1; })()""")
    stat = wait(r"""(() => {
      const row = [...document.querySelectorAll('tr')].find(tr => tr.innerText.includes('hashPassword'));
      if (!row) return false;
      if (row.innerText.includes('通过')) return 'active';
      if (row.innerText.includes('失败')) return 'error';
      return 'waiting';
    })()""", 45, "探针 active")
    check("探针 active（同方法位互斥下唯一挂载）", stat == "active", str(stat))

    # REPL 触发两次（首调走原实现：ART 去优化时机）
    js("""(() => { const t = [...document.querySelectorAll('.probe-tab')].find(x => x.innerText.includes('REPL')); if (t) t.click(); return 'ok'; })()""")
    time.sleep(0.8)
    check("填 REPL 表达式", fill_input("Java.use", 'Java.use("com.notevault.app.utils.AESUtil").hashPassword("Wei123123")') == "filled")
    click_button("执行")
    repl_out = wait(r"""(() => {
      const m = document.body.innerText.match(/\[str\] ([A-Za-z0-9+/=]{20,})/);
      return m ? m[1] : false;
    })()""", 30, "REPL 输出")
    check("REPL 调用 hashPassword 返回 Base64 串", isinstance(repl_out, str) and len(repl_out) > 20, str(repl_out)[:60])
    click_button("执行")  # 第二次：探针必命中
    time.sleep(2)
    shot("it_04_probe_repl")

    print("== 5. 时间轴：命中可见 + 暂停冻结 ==")
    js("""(() => { const pinia = document.querySelector('#app').__vue_app__.config.globalProperties.$pinia; pinia._s.get('app').switchView('timeline'); return 'ok'; })()""")
    time.sleep(1.2)
    rows = wait(r"""(() => { const m = document.body.innerText.match(/(\d+) 条（内存保留/); return m ? Number(m[1]) : 0; })()""",
                20, "时间轴行数")
    check("时间轴显示命中事件", isinstance(rows, int) and rows >= 1, f"{rows} 条")
    click_button("暂停")
    paused = wait(r"""(() => document.body.innerText.includes('已暂停') ? true : false)()""", 10, "暂停")
    check("暂停冻结生效", paused is True)
    click_button("继续")
    shot("it_05_timeline")

    print("== 6. 韧性：taskkill sidecar → 会话失败 → 重新附加恢复（forward_events 重订阅）===")
    import subprocess
    subprocess.run(["taskkill", "/F", "/IM", "frida_bridge.exe"], capture_output=True)
    time.sleep(2)
    fail_seen = wait(r"""(() => {
      const pinia = document.querySelector('#app').__vue_app__.config.globalProperties.$pinia;
      const st = pinia.state.value.session || {};
      const phase = st.session && st.session.phase;
      const msgs = (st.messages || []).map(m => m.text).join(' ');
      return (phase === 'failed' || msgs.includes('connectionTerminated')) ? 'seen' : false;
    })()""", 30, "分离/失败信号")
    check("sidecar 死亡被感知（失败/终止信号出现）", fail_seen == "seen")
    node(2)
    click_button("枚举进程")
    wait(r"""(() => { const m = document.body.innerText.match(/共 (\d+) 条/); return m ? Number(m[1]) >= 1 : false; })()""",
         60, "重启后进程枚举（sidecar 自动重启）")
    r = js(r"""(() => {
      const rows = [...document.querySelectorAll('tr')];
      const target = rows.find(tr => /notevault/i.test(tr.innerText) && tr.querySelector('button'));
      if (target) { const b = [...target.querySelectorAll('button')].find(b => b.innerText.includes('附加')); if (b) { b.click(); return 'attached'; } }
      return 'not-found';
    })()""")
    check("重启后重新附加", r == "attached", str(r))
    wait(r"""(() => document.body.innerText.includes('hello：frida') || document.body.innerText.includes('hello 握手成功') ? true : false)()""",
         60, "重启后 hello")
    check("重启后 hello 握手成功（事件管线自愈验证通过）", True)
    shot("it_06_recover")

    print("== 7. 还原节点：U4 真案（notevault Wei123123）==")
    node(6)
    target_b64 = "1Q0Tvj9/GLaF+cTMH+2c8z8ieGV3n4L+jySUZLWBcUc="
    assert fill_sample(0, "Wei123123", "etmLYLvSIJn2mzgC", target_b64) == "filled", "样本行填写失败"
    click_button("运行还原")
    desc = wait(r"""(() => {
      const el = document.querySelector('.scheme-result .mono');
      return el ? el.innerText : false;
    })()""", 300, "穷举还原出方案")
    check("U4 真案还原：SHA-256 族命中", "SHA-256" in str(desc), str(desc)[:100])
    check("还原自证通过（数学自证 + 双证据）", js("""(() => document.body.innerText.includes('自证') ? document.body.innerText.includes('True') || document.body.innerText.includes('通过') : false)()""") is True)
    shot("it_07_restore")

    print("== 8. 爆破编排（合成小空间，U5 路径）==")
    import hashlib
    salt = "somesalt"
    t1 = hashlib.sha256(b"5937" + salt.encode()).hexdigest()
    # 爆破基于 recResult 的方案：先用合成案重跑还原，让方案与爆破样本同源
    # （此前直接沿用 U4 的 base64 方案 + hex 样本 → 自测门按设计拒绝，C-07）
    check("合成案样本填写", fill_sample(0, "5937", salt, t1) == "filled")
    click_button("运行还原")
    wait(r"""(() => {
      const el = document.querySelector('.scheme-result .mono');
      return el && el.innerText.includes("SHA-256") ? el.innerText.slice(0, 40) : false;
    })()""", 120, "合成案还原")
    check("合成案方案就绪（SHA-256 hex）", True)
    check("填掩码", fill_input("掩码", "?d?d?d?d") == "filled")
    check("填盐", fill_input("盐（同还原时）", salt) == "filled")
    check("填自测明文", fill_input("自测明文", "5937") == "filled")
    # 目标哈希由还原样本带入？爆破需要已知样本：m4 表单依赖还原结果；此处直接用还原节点的样本已换。
    # 回填样本行为 U5 合成案（5937/somesalt/sha256），再预估+内置爆破
    assert fill_sample(0, "5937", salt, t1) == "filled"
    click_button("预估")
    wait(r"""(() => document.body.innerText.includes('总数') ? true : false)()""", 30, "预估三件套")
    click_button("内置爆破")
    hit = wait(r"""(() => {
      const m = document.body.innerText.match(/pwd=([0-9a-zA-Z]+)/);
      return m ? m[1] : false;
    })()""", 180, "HIT 命中")
    check("内置爆破 HIT 5937（U5 路径）", hit == "5937", str(hit))
    shot("it_08_brute")

    print("== 9. 台账/案卷 ==")
    js("""(() => { const pinia = document.querySelector('#app').__vue_app__.config.globalProperties.$pinia; pinia._s.get('app').switchView('ledger'); return 'ok'; })()""")
    ledger_ok = wait(r"""(() => document.body.innerText.includes('发现台账') || document.body.innerText.includes('案卷') ? 'yes' : false)()""", 15, "台账视图")
    check("台账视图渲染（登记发现 + 发现台账/案卷）", ledger_ok == "yes")
    shot("it_09_ledger")

    print("\n========== 层2 GUI 联调结果 ==========")
    fails = [r for r in results if not r[1]]
    for name, ok, detail in results:
        print(f"{'✔' if ok else '✖'} {name}" + (f" —— {detail}" if detail else ""))
    print(f"\n{len(results) - len(fails)}/{len(results)} 项通过")
    return 1 if fails else 0


if __name__ == "__main__":
    sys.exit(main())
