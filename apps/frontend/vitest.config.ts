/// <reference types="vitest/config" />
import path from "node:path";
import react from "@vitejs/plugin-react";
import tsconfigPaths from "vite-tsconfig-paths";
import { defineConfig } from "vitest/config";
import { fileURLToPath } from 'node:url';
import { storybookTest } from '@storybook/addon-vitest/vitest-plugin';
const dirname = typeof __dirname !== 'undefined' ? __dirname : path.dirname(fileURLToPath(import.meta.url));

// More info at: https://storybook.js.org/docs/next/writing-tests/integrations/vitest-addon
export default defineConfig({
  plugins: [react(), tsconfigPaths()],
  test: {
    environment: "jsdom",
    globals: true,
    setupFiles: [path.resolve(__dirname, "./src/test-utils/setup.ts")],
    include: ["src/**/*.test.{ts,tsx}", "src/**/*.unit.test.{ts,tsx}", "src/**/*.integration.test.{ts,tsx}", "src/**/*.e2e.test.{ts,tsx}", "src/**/__tests__/**/*.{ts,tsx}"],
    exclude: ["**/test-helpers.ts", "**/test-helpers.tsx", "**/node_modules/**"],
    coverage: {
      include: ["src/**/*.{ts,tsx}"],
      exclude: ["src/test-utils/**", "src/**/*.test.{ts,tsx}", "src/**/__tests__/**", "src/types/**", "src/main.tsx", "src/routeTree.gen.ts"]
    },
    pool: "forks",
    poolOptions: {
      forks: {
        singleFork: true
      }
    },
    projects: [{
      extends: true,
      plugins: [
      // The plugin will run tests for the stories defined in your Storybook config
      // See options at: https://storybook.js.org/docs/next/writing-tests/integrations/vitest-addon#storybooktest
      storybookTest({
        configDir: path.join(dirname, '.storybook')
      })],
      test: {
        name: 'storybook',
        browser: {
          enabled: true,
          headless: true,
          provider: 'playwright',
          instances: [{
            browser: 'chromium'
          }]
        },
        setupFiles: ['.storybook/vitest.setup.ts']
      }
    }]
  },
  resolve: {
    alias: {
      "@": path.resolve(__dirname, "./src"),
      "@moatless/types": path.resolve(__dirname, "../../packages/types/src"),
      "@test-utils": path.resolve(__dirname, "./src/test-utils")
    }
  },
  define: {
    "process.env.NODE_ENV": JSON.stringify("test")
  }
});