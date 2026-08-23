import { defineConfig, mergeConfig } from "vitest/config";
import viteConfig from "./vite.config";

export default defineConfig(async (env) => {
  const baseConfig =
    typeof viteConfig === "function" ? await viteConfig(env) : viteConfig;

  return mergeConfig(baseConfig, {
    test: {
      environment: "jsdom",
      pool: "threads",
      maxWorkers: 1,
      globals: true,
      setupFiles: ["./src/test/setup.ts"],
      include: ["src/**/*.{test,spec}.{ts,tsx}"],
      passWithNoTests: true,
    },
  });
});
