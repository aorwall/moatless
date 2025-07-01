import { defineConfig, devices } from '@playwright/test'

/**
 * Playwright configuration for infrastructure testing only
 * This runs basic validation tests without requiring a full server setup
 */
export default defineConfig({
  testDir: './tests/e2e',
  testMatch: ['**/infrastructure-test.spec.ts', '**/smoke.spec.ts'],
  
  /* Run tests in files in parallel */
  fullyParallel: true,
  
  /* Fail the build on CI if you accidentally left test.only in the source code. */
  forbidOnly: !!process.env.CI,
  
  /* Retry on CI only */
  retries: process.env.CI ? 2 : 0,
  
  /* Opt out of parallel tests on CI. */
  workers: process.env.CI ? 1 : undefined,
  
  /* Reporter to use. See https://playwright.dev/docs/test-reporters */
  reporter: 'html',
  
  /* Shared settings for all the projects below. */
  use: {
    /* Collect trace when retrying the failed test. */
    trace: 'on-first-retry',
    
    /* Take screenshot on failure */
    screenshot: 'only-on-failure',
  },

  /* Configure projects for major browsers */
  projects: [
    {
      name: 'chromium',
      use: { ...devices['Desktop Chrome'] },
    },
  ],

  /* Simple setup and teardown for infrastructure tests */
  globalSetup: './tests/e2e/helpers/simple-global-setup.ts',
  globalTeardown: './tests/e2e/helpers/simple-global-teardown.ts',

  /* Configure timeouts */
  timeout: 30000, // 30 seconds for infrastructure tests
  expect: {
    timeout: 5000, // 5 seconds for assertions
  },

  /* Configure test output directory */
  outputDir: 'test-results/',
})