/** 运行环境判定：浏览器预览（vite dev 直开）时 Tauri IPC 不可用 */
export function isTauri(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

export function platform(): string {
  return navigator.platform || "unknown";
}
