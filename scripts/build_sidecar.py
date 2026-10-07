"""通道B sidecar 打包（文档10 P4-1）：PyInstaller onefile 产出 sidecar/dist/frida_bridge.exe
并拍平复制为 sidecar/frida_bridge.exe（tauri resources 引用拍平路径，见批次⑧出货级修复）。

用途：单安装包分发的零外部依赖目标——sidecar 捆绑 Python + frida 客户端，
安装后无需系统 Python / pip install frida（通道C 仍可选 frida CLI）。

用法（构建机一次性准备：pip install pyinstaller，frida 需与 bin/frida-server 矩阵同版本）：
    python scripts/build_sidecar.py
产物：sidecar/dist/frida_bridge.exe（tauri bundle resources 会把它映射为 sidecar/frida_bridge.exe）

注意：
- 必须保留控制台（stdio JSON-RPC 是通道B 的传输层），禁止 --noconsole/--windowed。
- frida 客户端版本必须与 bin/frida-server/<版本> 矩阵一致（S-01 三处一致）。
"""

from __future__ import annotations

import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
DIST = ROOT / "sidecar" / "dist"
WORK = ROOT / "build" / "sidecar"
SPEC = ROOT / "build" / "sidecar"


def main() -> int:
    try:
        import PyInstaller  # noqa: F401
    except ImportError:
        print("[sidecar] 缺少 pyinstaller：pip install pyinstaller", file=sys.stderr)
        return 2

    # 版本自检：frida 客户端 vs 随包矩阵（S-01）
    try:
        client = subprocess.run(
            [sys.executable, "-c", "import frida; print(frida.__version__)"],
            capture_output=True, text=True, check=True,
        ).stdout.strip()
    except Exception as e:  # noqa: BLE001
        print(f"[sidecar] frida 模块不可用：{e}", file=sys.stderr)
        return 2
    matrix = sorted((ROOT / "bin" / "frida-server").glob("*"))
    versions = [d.name for d in matrix if d.is_dir()]
    if client not in versions:
        print(
            f"[sidecar] ★S-01 警告：客户端 frida {client} 不在随包矩阵 {versions} 中，"
            "推送/安装链将无法匹配版本——请先对齐（pip install frida==<矩阵版本>）",
            file=sys.stderr,
        )
        return 2
    print(f"[sidecar] frida 客户端 {client} 与随包矩阵一致")

    import PyInstaller.__main__

    DIST.mkdir(parents=True, exist_ok=True)
    WORK.mkdir(parents=True, exist_ok=True)
    PyInstaller.__main__.run(
        [
            str(ROOT / "sidecar" / "frida_bridge.py"),
            "--onefile",
            "--name", "frida_bridge",
            "--distpath", str(DIST),
            "--workpath", str(WORK),
            "--specpath", str(SPEC),
            # stdio JSON-RPC 是通道B 传输层：必须保留控制台
            "--console",
            # 体积瘦身（批次⑬）：sidecar 只用 stdlib + frida + concurrent.futures，
            # 这些标准库/随 pip 附带的模块永远用不到，剔除后 onefile 解压更快
            # （懒启动场景解压延迟直接计入用户首次操作）。注意不可剔
            # concurrent.futures（批次⑩起 frida_bridge.py 的工作池依赖它）。
            "--exclude-module", "tkinter",
            "--exclude-module", "unittest",
            "--exclude-module", "pydoc_data",
            "--exclude-module", "lib2to3",
            "--exclude-module", "setuptools",
            "--exclude-module", "distutils",
            "--exclude-module", "xmlrpc",
            "--clean",
            "--noconfirm",
        ]
    )
    out = DIST / "frida_bridge.exe"
    if not out.is_file():
        print("[sidecar] 构建失败：未产出 frida_bridge.exe", file=sys.stderr)
        return 1
    # 拍平复制到 sidecar/frida_bridge.exe：tauri resources 用 ../sidecar/frida_bridge.exe，
    # 安装后落在 _up_/sidecar/frida_bridge.exe，resource_join("sidecar/frida_bridge.exe")
    # 的两级定位才能命中（dist 子目录会让安装版永远找不到 sidecar exe——批次⑧出货级修复）
    import shutil
    flat = ROOT / "sidecar" / "frida_bridge.exe"
    shutil.copy2(out, flat)
    print(f"[sidecar] 完成：{out} → {flat}（{out.stat().st_size / 1024 / 1024:.1f} MB）")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
