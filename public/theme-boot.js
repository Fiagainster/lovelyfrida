// 内联启动着色：消除白屏与主题闪烁（LovelyMem 模式）。
// 批次⑮：从 index.html 内联脚本外置为本文件——CSP 启用 script-src 'self' 后
// 内联脚本会被拦截，外置同源脚本既保住首帧着色又满足严格 CSP。
(function () {
  try {
    var t = localStorage.getItem("lf.theme") || "dark";
    document.documentElement.setAttribute("data-theme", t);
  } catch {
    document.documentElement.setAttribute("data-theme", "dark");
  }
})();
