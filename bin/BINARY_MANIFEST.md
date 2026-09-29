# 二进制清单（供应链审计，文档07要求）

> 全部内置二进制必须登记：名称 / 版本 / sha256 / 下载来源。应用首次启动自检时逐一核对 sha256。

## adb（Windows x64）

| 文件 | 版本 | sha256 | 来源 |
|---|---|---|---|
| adb.exe | 1.0.41 (platform-tools) | `56656270da132f44e9cb4fb86a12ba965635c80423d43dcdd944d9fec4ab4622` | 本机 Android SDK `D:\System\AndroidSdk\platform-tools\`（google 官方 platform-tools 分发） |
| AdbWinApi.dll | — | `a00cf631dd12c82561ffccefdb1a99a27c527253b04f06ca8fc1bb86ba2148c4` | 同上 |
| AdbWinUsbApi.dll | — | `e4d72d5ba3bf4b027f1b7a3781aae0b04712c1a52244bea277fb57dd75a85702` | 同上 |

> 注：Doctor 运行期探测遵循「模拟器自带 adb 优先」（E-04，如 `D:\System\MuMu\MuMuPlayer\nx_main\adb.exe`）；`bin\adb` 是随包分发的兜底版本。

## frida-server（Android 设备端，版本矩阵，三处一致原则）

| 版本 | ABI | sha256 | 来源 | 状态 |
|---|---|---|---|---|
| 16.7.19 | android-x86_64 | — | https://github.com/frida/frida/releases/tag/16.7.19 | M1 下载 |
| 16.7.19 | android-arm64 | — | 同上 | M1 下载 |
| 17.19.0 | android-x86_64 | — | https://github.com/frida/frida/releases/tag/17.19.0 | M1 下载 |
| 17.19.0 | android-arm64 | — | 同上 | M1 下载 |

> 客户端版本矩阵（三处一致，S-01）：本机 pip `frida==17.19.0`（已装，frida-tools 14.10.4）；sidecar wheel 矩阵随包（M1）；设备端 frida-server 上表。

## Python sidecar（M1 落地）

| 组件 | 版本 | 来源 | 状态 |
|---|---|---|---|
| python embeddable | 3.13.x | python.org Windows embeddable zip | M1 |
| frida wheel | ==17.19.0（与设备端矩阵一致） | PyPI | M1 |
