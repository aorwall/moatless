import { defineConfig, devices } from "@playwright/test";

/**
 * See https://playwright.dev/docs/test-configuration.
 */
export default defineConfig({
	testDir: "./tests/e2e",
	/* Run tests in files in parallel */
	fullyParallel: true,
	/* Fail the build on CI if you accidentally left test.only in the source code. */
	forbidOnly: !!process.env.CI,
	/* Retry on CI only */
	retries: process.env.CI ? 2 : 0,
	/* Opt out of parallel tests on CI. */
	workers: process.env.CI ? 1 : undefined,
	/* Reporter to use. See https://playwright.dev/docs/test-reporters */
	reporter: "html",
	/* Shared settings for all the projects below. See https://playwright.dev/docs/api/class-testoptions. */
	use: {
		/* Base URL to use in actions like `await page.goto('/')`. */
		baseURL: process.env.E2E_BASE_URL || "http://127.0.0.1:3002",

		/* Collect trace when retrying the failed test. See https://playwright.dev/docs/trace-viewer */
		trace: "on-first-retry",

		/* Take screenshot on failure */
		screenshot: "only-on-failure",

		/* Record video on failure */
		video: "retain-on-failure",
	},

	/* Configure projects for major browsers */
	projects: [
		{
			name: "chromium",
			use: { ...devices["Desktop Chrome"] },
		},

		/* Other browsers disabled - Chrome only for E2E tests */
		// {
		//   name: 'firefox',
		//   use: { ...devices['Desktop Firefox'] },
		// },
		// {
		//   name: 'webkit',
		//   use: { ...devices['Desktop Safari'] },
		// },
	],

	/* Configure global setup and teardown */
	globalSetup: "./tests/e2e/helpers/global-setup.ts",
	globalTeardown: "./tests/e2e/helpers/global-teardown.ts",

	/* Configure timeouts */
	timeout: 90000, // 90 seconds for tests involving real agents
	expect: {
		timeout: 10000, // 10 seconds for assertions
	},

	/* Configure test output directory */
	outputDir: "test-results/",
});
