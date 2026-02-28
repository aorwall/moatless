import { defineConfig } from "@playwright/test";

/**
 * Playwright configuration for Docker-compose E2E tests.
 *
 * Key differences from main playwright.config.ts:
 * - NO global setup/teardown (uses running docker-compose environment)
 * - Single worker (tests may share resources and should run sequentially)
 * - Longer timeouts for container operations
 * - Separate output directory for test results
 *
 * Prerequisites:
 * 1. Start docker-compose: cd docker-compose && ./dev-docker.sh
 * 2. Run tests: bun run test:e2e:docker
 */
export default defineConfig({
	testDir: "./tests/e2e/docker/specs",

	// Sequential execution for Docker tests (tests may share resources)
	fullyParallel: false,

	// Fail the build on CI if you accidentally left test.only in source code
	forbidOnly: !!process.env.CI,

	// No retries for Docker tests (each test should be deterministic)
	retries: 0,

	// Single worker for Docker tests to avoid resource conflicts
	workers: 1,

	// Reporter configuration
	reporter: [
		["html", { outputFolder: "test-results/docker-e2e" }],
		["json", { outputFile: "test-results/docker-e2e/results.json" }],
		["list"],
	],

	// Shared settings for all tests
	use: {
		// Base URL for API requests - docker-compose backend
		baseURL: process.env.DOCKER_COMPOSE_URL || "http://localhost:8080",

		// Collect trace when retrying the failed test
		trace: "on-first-retry",

		// Take screenshot on failure
		screenshot: "only-on-failure",

		// Record video on failure
		video: "retain-on-failure",

		// Extra HTTP headers for API requests
		extraHTTPHeaders: {
			Accept: "application/json",
		},
	},

	// Single project for Docker API tests
	projects: [
		{
			name: "docker-api",
			use: {},
		},
	],

	// Longer timeouts for Docker operations
	// 3 minutes for tests (container provisioning can be slow)
	timeout: 180000,

	// 30 seconds for assertions
	expect: {
		timeout: 30000,
	},

	// Output directory for test artifacts
	outputDir: "test-results/docker-e2e",

	// NO global setup/teardown - tests run against existing docker-compose
	// globalSetup: undefined,
	// globalTeardown: undefined,
});
