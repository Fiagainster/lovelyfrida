#!/usr/bin/env python3
"""M1 验收（健壮版）：每次求值独立建立 CDP 连接，避免长连接状态漂移。"""
import sys
import time

from cdp_common import click_button, js, shot, wait


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
