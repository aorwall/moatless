import { config } from "dotenv";
import { resolve } from "node:path";
import { defineConfig } from "@playwright/test";

// Load env vars from root .env
config({ path: resolve(__dirname, "../../.env") });

/**
 * Playwright configuration for browser-based UI E2E tests.
 *
 * Placeholder — browser tests will be added here.
 *
 * Prerequisites:
 *   Start docker-compose: cd docker-compose && ./dev-docker.sh
 *   Run: npx playwright test --config tests/ui/playwright.config.ts
 */
export default defineConfig({
	testDir: "./specs",

	fullyParallel: false,

	workers: process.env.CI ? 1 : 2,

	forbidOnly: !!process.env.CI,

	retries: process.env.CI ? 1 : 0,

	// 3 minute timeout for UI tests
	timeout: 180_000,

	expect: {
		timeout: 15_000,
	},

	reporter: [
		["list"],
		["html", { outputFolder: "../../test-results/ui" }],
	],

	use: {
		baseURL:
			process.env.DOCKER_COMPOSE_URL ||
			`http://localhost:${process.env.BACKEND_PORT || "8080"}`,

		trace: "on-first-retry",
		screenshot: "only-on-failure",
		video: "retain-on-failure",
	},

	projects: [
		{
			name: "chromium",
			use: { browserName: "chromium" },
		},
	],

	outputDir: "../../test-results/ui",
});
