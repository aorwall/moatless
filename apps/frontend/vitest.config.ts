/// <reference types="vitest/config" />
import path from "node:path";
import react from "@vitejs/plugin-react";
import tsconfigPaths from "vite-tsconfig-paths";
import { defineConfig } from "vitest/config";

export default defineConfig({
  plugins: [react(), tsconfigPaths()],
  test: {
    environment: "jsdom",
    globals: true,
    setupFiles: [path.resolve(__dirname, "./src/test-utils/setup.ts")],
    server: {
      deps: {
        // Inline mermaid to properly handle its CSS imports
        inline: ['mermaid', 'katex']
      }
    },
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
      plugins: []
    }]
  },
  resolve: {
    alias: {
      "@": path.resolve(__dirname, "./src"),
      "@moatless/types": path.resolve(__dirname, "../../packages/types/src"),
      "@test-utils": path.resolve(__dirname, "./src/test-utils"),
      // Stub CSS imports from node_modules (e.g., katex)
      "katex/dist/katex.min.css": path.resolve(__dirname, "./src/test-utils/css-stub.ts"),
      "katex/dist/katex.css": path.resolve(__dirname, "./src/test-utils/css-stub.ts")
    }
  },
  define: {
    "process.env.NODE_ENV": JSON.stringify("test")
  }
});