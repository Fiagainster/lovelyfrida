import { writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import net from "node:net";
import { spawn } from "node:child_process";

/**
 * dev 启动器：这台机器的 WinNAT 动态保留端口范围会漂移（1420 间歇性 EACCES），
 * 所以启动前先探测一个真正可绑定的端口，然后：
 *   1. 写 src-tauri/tauri.dev.conf.json 覆盖 build.devUrl
 *   2. 用 LF_DEV_PORT 环境变量把同一端口传给 vite（vite.config.ts 读取）
 * 用法：npm run dev:tauri
 */
const root = join(dirname(fileURLToPath(import.meta.url)), "..");

function tryBind(port) {
  return new Promise((resolve) => {
    const s = net.createServer();
    s.once("error", () => resolve(false));
    s.once("listening", () => s.close(() => resolve(true)));
    s.listen(port, "127.0.0.1");
  });
}

const candidates = [1420, 1421, 1422, 24100, 24142, 25142, 26142, 27142, 28120];
let port = null;
for (const p of candidates) {
  if (await tryBind(p)) {
    port = p;
    break;
  }
}
if (!port) {
  console.error("[dev] 候选端口全部被占（WinNAT 保留范围漂移），请手动指定一个空闲端口");
  process.exit(1);
}
console.log(`[dev] 使用端口 ${port}`);

const devConfPath = join(root, "src-tauri", "tauri.dev.conf.json");
writeFileSync(
  devConfPath,
  JSON.stringify({ build: { devUrl: `http://127.0.0.1:${port}` } }, null, 2),
);

const child = spawn(
  "npx",
  ["tauri", "dev", "--config", devConfPath.replace(/\\/g, "/")],
  {
    cwd: root,
    stdio: "inherit",
    shell: true,
    env: { ...process.env, LF_DEV_PORT: String(port) },
  },
);
child.on("exit", (code) => process.exit(code ?? 0));
