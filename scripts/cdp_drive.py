#!/usr/bin/env python3
"""通过 WebView2 CDP 驱动 LovelyFrida 真实 UI 做按钮级验收。

前置：应用以 WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS="--remote-debugging-port=9223" 启动。
CDP 监听在 [::1]:9223（IPv6）。截图输出到 logs/ui_shots/。
"""
import sys
import time

from cdp_common import CDP, page_ws, wait_js


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
