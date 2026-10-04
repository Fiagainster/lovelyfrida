@echo off
rem 联调用：以 python 模式跑 sidecar（LOVELYFRIDA_SIDECAR 指向本文件；stdin/stdout 直通）
python -u "%~dp0frida_bridge.py"
