#!/usr/bin/env python3
"""CI 二进制供给（C3）：按 bin/binary_manifest.json 下载随包二进制并逐个对账 sha256。

- adb 三件 ← dl.google.com/android/repository/platform-tools-latest-windows.zip
- frida-server 四 ABI ← github.com/frida/frida/releases（xz 压缩资产，解压后校验）

产物落 bin/（gitignore 内，与本地手工供给布局一致）；任何 sha256 不符即非零退出。
"""
import hashlib
import io
import json
import lzma
import sys
import urllib.request
import zipfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
BIN = ROOT / "bin"
MANIFEST = json.loads((BIN / "binary_manifest.json").read_text(encoding="utf-8"))

PLATFORM_TOOLS_URL = "https://dl.google.com/android/repository/platform-tools-latest-windows.zip"


def fetch(url: str) -> bytes:
    print(f"GET {url}")
    req = urllib.request.Request(url, headers={"User-Agent": "lovelyfrida-ci"})
    with urllib.request.urlopen(req, timeout=180) as r:
        return r.read()


def sha256(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def write_entry(rel_path: str, data: bytes, want: str) -> None:
    got = sha256(data)
    if got != want:
        print(f"✖ sha256 不符：{rel_path}\n  期望 {want}\n  实得 {got}")
        sys.exit(1)
    out = BIN / rel_path
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_bytes(data)
    print(f"ok {rel_path}（sha256 对账通过）")


def main() -> int:
    entries = MANIFEST["files"]

    # ① adb：platform-tools 一个 zip 出三个文件
    adb_entries = [e for e in entries if e["path"].startswith("adb/")]
    if adb_entries:
        with zipfile.ZipFile(io.BytesIO(fetch(PLATFORM_TOOLS_URL))) as zf:
            names = {n.split("/")[-1]: n for n in zf.namelist()}
            for e in adb_entries:
                fname = Path(e["path"]).name
                if fname not in names:
                    print(f"✖ platform-tools 包内未找到 {fname}")
                    return 1
                write_entry(e["path"], zf.read(names[fname]), e["sha256"])

    # ② frida-server：release 的 xz 资产，解压后校验
    for e in (x for x in entries if x["path"].startswith("frida-server/")):
        ver, abi = e["version"], e.get("abi")
        if not abi:
            print(f"✖ manifest 条目缺 abi：{e['path']}")
            return 1
        url = f"https://github.com/frida/frida/releases/download/{ver}/frida-server-{ver}-android-{abi}.xz"
        write_entry(e["path"], lzma.decompress(fetch(url)), e["sha256"])

    total = len(adb_entries) + sum(1 for x in entries if x["path"].startswith("frida-server/"))
    print(f"共供给 {total} 个二进制，sha256 全部对账通过")
    return 0


if __name__ == "__main__":
    sys.exit(main())
