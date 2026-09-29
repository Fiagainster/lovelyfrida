import { createApp } from "vue";
import { createPinia } from "pinia";
import App from "./App.vue";
import "./styles/theme.css";
import "./styles/layout.css";

// 失败不静默（文档01 原则 + 文档05）：任何启动期错误直接显示在窗口上，不允许黑屏
function showFatal(origin: string, detail: unknown) {
  const err = detail as Error;
  const el = document.createElement("pre");
  el.style.cssText =
    "position:fixed;left:12px;right:12px;bottom:12px;max-height:45vh;overflow:auto;z-index:99999;" +
    "background:#2b1216;color:#ffb4c0;border:1px solid #ef4444;border-radius:8px;padding:12px;" +
    "font:12px/1.6 Consolas,monospace;white-space:pre-wrap;margin:0";
  el.textContent = `[${origin}] ${err?.message ?? String(detail)}\n${err?.stack ?? ""}`;
  document.body.appendChild(el);
}

window.addEventListener("error", (e) => showFatal("window.error", e.error ?? e.message));
window.addEventListener("unhandledrejection", (e) => showFatal("unhandledrejection", e.reason));

const app = createApp(App);
const pinia = createPinia();
app.use(pinia);

// 开发模式暴露 store 调试钩子（浏览器预览时注入 mock 数据自测 UI）
if (import.meta.env.DEV) {
  import("@/stores/pipeline").then(({ usePipelineStore }) => {
    (window as unknown as Record<string, unknown>).__pipeline = usePipelineStore(pinia);
  });
}

app.config.errorHandler = (err, _instance, info) => showFatal(`vue(${info})`, err);
app.mount("#app");
