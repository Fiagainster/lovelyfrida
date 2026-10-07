import subprocess, sys, threading, time, os
from pathlib import Path
# 批次⑭：路径改为相对仓库根（此前硬编码绝对路径，换机器即失效）
ROOT = Path(__file__).resolve().parent.parent
EXE = str(ROOT / "sidecar" / "dist" / "frida_bridge.exe")
LOGP = str(ROOT / "logs" / "wrapper" / "relay.log")
Path(LOGP).parent.mkdir(parents=True, exist_ok=True)
LOG = open(LOGP, "ab", buffering=0)
def log(m):
    LOG.write((f"[{time.strftime('%H:%M:%S')}.{int(time.time()*1000)%1000:03d}] {m}\n").encode())
p = subprocess.Popen([EXE], stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
log(f"spawned exe pid={p.pid} cwd={os.getcwd()}")
def relay_in():
    try:
        while True:
            b = sys.stdin.buffer.readline()
            if not b:
                log("HOST_STDIN_EOF (len0)")
                try: p.stdin.close()
                except Exception: pass
                break
            log(f"IN  {len(b):7d}B {b[:90]!r}")
            p.stdin.write(b); p.stdin.flush()
    except Exception as e:
        log(f"IN_EXC {type(e).__name__} {e}")
def relay_out():
    try:
        while True:
            b = p.stdout.readline()
            if not b:
                log("EXE_STDOUT_EOF")
                sys.stdout.buffer.flush()
                break
            log(f"OUT {len(b):7d}B {b[:90]!r}")
            sys.stdout.buffer.write(b); sys.stdout.buffer.flush()
    except Exception as e:
        log(f"OUT_EXC {type(e).__name__} {e}")
def relay_err():
    try:
        while True:
            b = p.stderr.readline()
            if not b: log("EXE_STDERR_EOF"); break
            log(f"ERR {b[:150]!r}")
    except Exception as e:
        log(f"ERR_EXC {e}")
threading.Thread(target=relay_in, daemon=True).start()
threading.Thread(target=relay_out, daemon=True).start()
threading.Thread(target=relay_err, daemon=True).start()
code = p.wait()
log(f"EXE_EXITED code={code}")
sys.exit(code if code is not None and code > 0 else 0)
