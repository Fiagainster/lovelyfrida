// 批次⑬ lint 基线：vue3-recommended + typescript-eslint 推荐。
// 项目已有 vue-tsc strict 门禁（类型安全），这里兜的是类型抓不到的：
// hook 依赖、未使用变量逃逸、v-for key、事件清理纪律等结构性问题。
import pluginVue from "eslint-plugin-vue";
import tseslint from "typescript-eslint";

export default tseslint.config(
  { ignores: ["dist/**", "node_modules/**", "src-tauri/**", "agent/**", "参考/**", "*.cjs"] },
  ...tseslint.configs.recommended,
  ...pluginVue.configs["flat/recommended"],
  {
    files: ["**/*.vue"],
    languageOptions: { parserOptions: { parser: tseslint.parser } },
  },
  {
    rules: {
      // 纯格式类规则关闭：项目模板为手写风格，格式化交给未来 prettier 统一决策，
      // 不让 eslint 的模板排版偏好制造上千条 churn（结构性规则全保留）
      "vue/max-attributes-per-line": "off",
      "vue/singleline-html-element-content-newline": "off",
      "vue/multiline-html-element-content-newline": "off",
      "vue/html-self-closing": "off",
      "vue/html-indent": "off",
      "vue/html-closing-bracket-newline": "off",
      "vue/first-attribute-linebreak": "off",
      "vue/html-quotes": "off",
      // 项目约定：innerHTML 仅用于本地证据渲染等受控场景，交给人工审查而非一刀切
      "vue/no-v-html": "off",
      // 多词组件名对 views/ 下页面组件噪音过大（PipelineView 等已合规，PlaceholderView 例外）
      "vue/multi-word-component-names": "off",
      "@typescript-eslint/no-unused-vars": ["error", { argsIgnorePattern: "^_", varsIgnorePattern: "^_" }],
    },
  },
);
