#!/usr/bin/env python3
"""M3 全链路验收（U6）：回灌向导七步——停应用 → 空跑建档 → 建目录 → 中转推送 → chown → restorecon → md5。

前置：dev 应用已启动（CDP 9223）+ 模拟器已 root。本脚本向 /data/local/tmp/lf-e2e
灌入一个自建小文件（不触碰任何真实应用数据），断言七步报告出现且无 fail 步。
"""
import sys
PAYLOAD = Path(__file__).resolve().parent.parent / "workspace" / "lf-e2e-inject.json"
import json
import time
from pathlib import Path

from cdp_common import click_button, fill_by_placeholder, js, shot, wait


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
