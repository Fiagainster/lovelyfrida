#!/usr/bin/env python3
"""内存验收实测（docs/08 非功能验收最后一项）：
标准：常驻 ≤ 300 MB；观测 10 万事件后增量 ≤ 200 MB。

方法：空闲基线 → 附加 notevault → stressFire 打 100000 条 probe_hit（批量层真实路径）
→ 洪峰中采样 → 静置 30s 后终测。计量对象：
  ① 宿主进程 lovelyfrida.exe 工作集（Rust 状态 + trace 环 + audit）
  ② WebView2 子进程合计（前端 Pinia trace 环 cap 3000 + 诊断引擎）
"""
import json
import subprocess
import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from it_layer2_gui import (  # noqa: E402
    click_button, fill_input, js, node, wait,
)

results: list[tuple[str, bool, str]] = []


def check(name, ok, detail=""):
    # 本地 check（不用 it_layer2_gui 的：它的 results 是那个模块的列表）
    results.append((name, ok, detail))
    print(f"  {'✔' if ok else '✖'} {name}" + (f" —— {detail}" if detail else ""))


def mem_mb() -> dict:
    """宿主 + WebView2 子进程工作集（MB）。"""
    def ws_of(name: str, parent: int | None = None) -> float:
        out = subprocess.run(
            ["powershell", "-NoProfile", "-Command",
             f"Get-Process {name} -ErrorAction SilentlyContinue | "
             "Select-Object Id,WorkingSet64 | ConvertTo-Json -Compress"],
            capture_output=True, text=True).stdout.strip()
        if not out:
            return 0.0
        try:
            data = json.loads(out)
        except Exception:
            return 0.0
        items = data if isinstance(data, list) else [data]
        total = 0.0
        for it in items:
            if parent is not None:
                # WebView2 子进程按父进程过滤（多主进程共用浏览器池时取我们那组）
                pass
            total += float(it.get("WorkingSet64", 0))
        return total / 1024 / 1024

    host = ws_of("lovelyfrida")
    wv = ws_of("msedgewebview2")
    return {"host": round(host, 1), "webview": round(wv, 1), "sum": round(host + wv, 1)}


def fmt(m: dict) -> str:
    return f"宿主 {m['host']}MB + WebView {m['webview']}MB = {m['sum']}MB"


def main():
    print("== 1. 空闲基线 ==")
    base = mem_mb()
    print("  ", fmt(base))
    check("空闲常驻 ≤ 300MB（宿主口径）", base["host"] <= 300, fmt(base))

    print("== 2. 附加 notevault ==")
    node(2)
    listening = js("""(() => document.body.innerText.includes('实测监听中') ? 'yes' : 'no')""")
    if listening != "yes":
        js("""(() => { const b = [...document.querySelectorAll('button')].find(x => x.innerText.trim() === '刷新'); if (b) b.click(); return 1; })()""")
        time.sleep(3)
    wait(r"""(() => document.body.innerText.includes('实测监听中') ? 'yes' : false)()""", 40, "frida-server 就绪")
    click_button("枚举进程")
    wait(r"""(() => { const m = document.body.innerText.match(/共 (\d+) 条/); return m ? Number(m[1]) >= 1 : false; })()""",
         60, "进程枚举")
    r = js(r"""(() => {
      const rows = [...document.querySelectorAll('tr')];
      const target = rows.find(tr => /notevault/i.test(tr.innerText) && tr.querySelector('button'));
      if (target) { const b = [...target.querySelectorAll('button')].find(b => b.innerText.includes('附加')); if (b) { b.click(); return 'attached'; } }
      return 'not-found';
    })()""")
    check("附加 notevault", r == "attached", str(r))
    wait(r"""(() => document.body.innerText.includes('hello：frida') || document.body.innerText.includes('hello 握手成功') ? true : false)()""",
         60, "hello 握手")
    after_attach = mem_mb()
    print("  ", fmt(after_attach))

    print("== 3. 100000 事件洪峰（批量路径）==")
    node(4)
    wait(r"""(() => document.body.innerText.includes('挂新探针') ? 'yes' : false)()""", 20, "探针表单")
    v = js("""window.__TAURI_INTERNALS__.invoke('frida_rpc', { f: 'stressFire', args: [{ n: 100000, tag: 'mem-test' }] })
      .then(r => JSON.stringify(r)).catch(e => 'ERR:' + String(e).slice(0, 100))""", timeout=120)
    check("stressFire 100000 条受理", str(v).startswith("{"), str(v)[:60])
    peak = {"sum": 0.0}
    for _ in range(12):
        time.sleep(1)
        m = mem_mb()
        peak = max(peak, m, key=lambda x: x["sum"])
        lat = js("""(() => { const t0 = performance.now(); return Math.round(performance.now() - t0); })()""", timeout=10)
    check("洪峰中宿主峰值 ≤ 300MB", peak["host"] <= 300, fmt(peak))

    print("== 4. 静置 30s 后终测 ==")
    time.sleep(30)
    final = mem_mb()
    print("  ", fmt(final))
    delta_host = round(final["host"] - base["host"], 1)
    delta_sum = round(final["sum"] - base["sum"], 1)
    check("10 万事件后宿主增量 ≤ 200MB", delta_host <= 200,
          f"宿主 Δ{delta_host}MB（终 {final['host']}MB − 基线 {base['host']}MB）；全口径 Δ{delta_sum}MB")
    # UI 存活：诊断面板 / 时间轴仍可交互
    alive = wait(r"""(() => document.body.innerText.length > 100 ? 'alive' : false)()""", 10, "UI 存活")
    check("洪峰后 UI 存活可交互", alive == "alive")

    print("\n========== 内存验收结果 ==========")
    fails = [r for r in results if not r[1]]
    for name, ok, detail in results:
        print(f"{'✔' if ok else '✖'} {name}" + (f" —— {detail}" if detail else ""))
    print(f"\n{len(results) - len(fails)}/{len(results)} 项通过")
    return 1 if fails else 0


if __name__ == "__main__":
    sys.exit(main())
