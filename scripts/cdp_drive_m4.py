#!/usr/bin/env python3
"""M4 全链路验收（U4+U5 前半）：算法还原（双样本防假命中）→ 内置爆破 HIT → 诊断卡联动。

用可自证的样本：SHA-256('5937'+'somesalt') 与 SHA-256('5938'+'somesalt')。
还原应穷举出「SHA-256 / 明文‖盐 / 单轮 / hex」；爆破填 ?d?d?d?d 应 HIT pwd=5937。
"""
import sys
import hashlib
import time

from cdp_common import click_button, fill_by_placeholder, js, shot, wait


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
