import { defineConfig } from "vitest/config";

export default defineConfig({
	test: {
		coverage: {
			include: ["server/**/*.{ts,js}", "types/**/*.{ts,js}"],
			exclude: [
				"**/node_modules/**",
				"**/__tests__/**",
				"**/*.test.{ts,js}",
				"**/*.spec.{ts,js}",
				"**/dist/**",
				"**/index.ts",
			],
			provider: "v8",
			reporter: ["text", "json", "html"],
			reportsDirectory: "./coverage",
		},
		globals: true,
		environment: "node",
		include: ["**/__tests__/**/*.test.{ts,js}", "**/*.test.{ts,js}"],
		exclude: ["**/node_modules/**", "**/dist/**", "**/client/**"],
		// Test timeouts to prevent hanging
		testTimeout: 10000, // 10s default
		// Concurrent execution for independent tests
		pool: "threads",
		poolOptions: {
			threads: {
				singleThread: false,
			},
		},
	},
	resolve: {
		alias: {
			"@shared": new URL("./types", import.meta.url).pathname,
			"@server": new URL("./server", import.meta.url).pathname,
		},
	},
});
