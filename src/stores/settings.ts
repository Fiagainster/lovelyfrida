import { defineStore } from "pinia";
import { ref, watch } from "vue";
import { api, type AppConfig } from "@/api";

const THEME_KEY = "lf.theme";

/**
 * 设置：Rust config.toml 是唯一真源（文档06）；
 * 前端镜像缓存 + localStorage 主题兜底（首帧内联着色依赖它）。
 */
export const useSettingsStore = defineStore("settings", () => {
  const config = ref<AppConfig | null>(null);
  const loaded = ref(false);
  const loadError = ref<string | null>(null);

  const theme = ref<"dark" | "light">(
    (localStorage.getItem(THEME_KEY) as "dark" | "light") || "dark",
  );

  async function load() {
    try {
      config.value = await api.getConfig();
      if (config.value.ui.theme !== theme.value) {
        theme.value = config.value.ui.theme;
      }
      loaded.value = true;
    } catch (e) {
      loadError.value = String(e);
    }
  }

  async function save() {
    if (!config.value) return;
    config.value = { ...config.value, ui: { ...config.value.ui, theme: theme.value } };
    try {
      config.value = await api.updateConfig(config.value);
    } catch (e) {
      // 浏览器预览时静默；真实运行失败要暴露
      if (!String(e).includes("浏览器预览")) throw e;
    }
  }

  function setTheme(t: "dark" | "light") {
    theme.value = t;
  }

  watch(theme, (t) => {
    document.documentElement.setAttribute("data-theme", t);
    localStorage.setItem(THEME_KEY, t);
    void save();
  });

  return { config, loaded, loadError, theme, load, save, setTheme };
});
