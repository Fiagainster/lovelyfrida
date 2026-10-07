#!/usr/bin/env python3
"""run_acceptance.py — 验收统一入口（批次⑮）：一条命令复跑全套/单层回归。

分层（与 docs/10 批次⑦⑩⑪ 验收脚本一一对应）：
  layer1   协议层 18 项（scripts/it_layer1_sidecar.py，需 MuMu + root + frida-server）
  layer2   CDP 全业务链（scripts/it_layer2_gui.py，需应用以 CDP 9223 启动 + 模拟器）
  stress   O-02 事件压测（scripts/it_stress_events.py，需真机）
  memory   内存验收（scripts/it_memory_check.py，需应用运行）
  lifecycle sidecar 请求生命周期冒烟（scripts/it_sidecar_lifecycle.py，免真机，CI 可跑）
  all      依次跑 lifecycle → （有设备/应用时手动再跑其余，因前置不同默认不含）

用法：
  python scripts/run_acceptance.py                # 免真机组（lifecycle）
  python scripts/run_acceptance.py layer1 layer2  # 指定层（前置自备）
  python scripts/run_acceptance.py all            # 全部（中途失败即停）
"""
from __future__ import annotations

import subprocess
import sys
from pathlib import Path

SCRIPTS = Path(__file__).resolve().parent

LAYERS: dict[str, list[str]] = {
    "lifecycle": ["python", str(SCRIPTS / "it_sidecar_lifecycle.py")],
    "layer1": ["python", str(SCRIPTS / "it_layer1_sidecar.py")],
    "layer2": ["python", str(SCRIPTS / "it_layer2_gui.py")],
    "stress": ["python", str(SCRIPTS / "it_stress_events.py")],
    "memory": ["python", str(SCRIPTS / "it_memory_check.py")],
}


def main() -> int:
    args = sys.argv[1:] or ["lifecycle"]
    if "all" in args:
        targets = list(LAYERS)
    else:
        unknown = [a for a in args if a not in LAYERS]
        if unknown:
            print(f"未知层：{unknown}；可选：{list(LAYERS)} 或 all", file=sys.stderr)
            return 2
        targets = args
    failed: list[str] = []
    for t in targets:
        print(f"\n===== 验收层 {t}：{' '.join(LAYERS[t])} =====")
        r = subprocess.run(LAYERS[t])
        if r.returncode != 0:
            failed.append(t)
            print(f"===== {t} ❌（exit {r.returncode}）=====")
            if "all" not in args:
                continue
            break
        print(f"===== {t} ✅ =====")
    if failed:
        print(f"\n结果：{len(targets) - len(failed)} 过 / {len(failed)} 败（{failed}）")
        return 1
    print(f"\n结果：{len(targets)} 层全过 ✅")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
