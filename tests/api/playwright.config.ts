import { config } from "dotenv";
import { resolve } from "node:path";
import { defineConfig } from "@playwright/test";

// Load env vars from root .env
config({ path: resolve(__dirname, "../../.env") });

/**
 * Playwright configuration for API-only E2E tests.
 *
 * No browser — uses native fetch for all API calls.
 * Chains run serially within a describe block; chains can parallelize across workers.
 *
 * Prerequisites:
 *   Start docker-compose: cd docker-compose && ./dev-docker.sh
 *   Run: GITHUB_PAT=ghp_xxx npx playwright test --config tests/api/playwright.config.ts
 */
export default defineConfig({
	testDir: "./chains",

	// Global setup: validate env vars, reset database
	globalSetup: resolve(__dirname, "./global-setup.ts"),
	globalTeardown: resolve(__dirname, "./global-teardown.ts"),

	// Chains are serial internally; this allows separate chains to parallelize
	fullyParallel: false,

	// Workers: 1 on CI (safer), 3 locally (chains are independent)
	workers: process.env.CI ? 1 : 3,

	// Fail the build on CI if you accidentally left test.only in source code
	forbidOnly: !!process.env.CI,

	// No retries — each chain should be deterministic
	retries: 0,

	// 5 minute timeout for chains involving agent execution
	timeout: 300_000,

	// 30 second assertion timeout
	expect: {
		timeout: 30_000,
	},

	// Reporter configuration
	reporter: [
		["list"],
		["html", { outputFolder: "../../test-results/api" }],
		["json", { outputFile: "../../test-results/api/results.json" }],
	],

	// Shared settings
	use: {
		baseURL:
			process.env.DOCKER_COMPOSE_URL ||
			`http://localhost:${process.env.BACKEND_PORT || "8080"}`,

		// No browser features
		trace: "off",
		screenshot: "off",
		video: "off",
	},

	// Single project — no browser projects
	projects: [{ name: "api" }],

	// Output directory for test artifacts
	outputDir: "../../test-results/api",
});
