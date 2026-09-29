import { defineConfig } from "vite";
import vue from "@vitejs/plugin-vue";
import { fileURLToPath, URL } from "node:url";

// Tauri 约定：固定端口、非浏览器环境变量前缀
export default defineConfig({
  plugins: [vue()],
  resolve: {
    alias: {
      "@": fileURLToPath(new URL("./src", import.meta.url)),
    },
  },
  clearScreen: false,
  server: {
    host: "127.0.0.1", // 显式 IPv4，避免 localhost 解析到 ::1 与 WebView2 不一致
    // 端口由 scripts/dev.mjs 探测后通过 LF_DEV_PORT 传入（WinNAT 保留范围漂移规避）
    port: Number(process.env.LF_DEV_PORT || 1420),
    strictPort: true,
  },
  envPrefix: ["VITE_", "TAURI_"],
  build: {
    target: "chrome105",
    minify: "esbuild",
    sourcemap: false,
    chunkSizeWarningLimit: 4096,
  },
});
