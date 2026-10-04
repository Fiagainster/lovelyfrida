import { defineConfig } from "vitest/config";

/** agent 层单测：只测可脱离 frida 运行时的纯逻辑（Node 环境；真机行为归 it_layer1 驱动） */
export default {
  test: {
    environment: "node",
    include: ["tests/**/*.test.ts"],
  },
} as const;
