import { resolve } from 'node:path'
import { defineConfig } from '@playwright/test'
import { profile } from '../shared/profile'

/** Written by tests/ui/auth.setup.ts, consumed by the chromium project. */
const STORAGE_STATE = resolve(__dirname, '../../test-results/ui/.auth/admin.json')

// A local run keeps its settings in the root .env; a sandbox run already has
// them in its environment, so a missing file is not an error.
try {
  process.loadEnvFile(resolve(__dirname, '../../.env'))
} catch {
  // No .env — the environment is expected to carry the settings already.
}

/**
 * Playwright configuration for browser-based UI E2E tests.
 *
 * The `setup` project signs in through the login form and saves the session to
 * `storageState`; the `chromium` project reuses it.
 *
 * Prerequisites:
 *   Start docker-compose: cd docker-compose && ./dev-docker.sh
 *   Run: npx playwright test --config tests/ui/playwright.config.ts
 */
export default defineConfig({
  testDir: '.',

  fullyParallel: false,

  workers: process.env.CI ? 1 : 2,

  forbidOnly: !!process.env.CI,

  retries: process.env.CI ? 1 : 0,

  // 3 minute timeout for UI tests
  timeout: 180_000,

  expect: {
    timeout: 15_000,
  },

  reporter: [['list'], ['html', { outputFolder: '../../test-results/ui' }]],

  use: {
    baseURL: profile().baseUrl,

    trace: 'on-first-retry',
    screenshot: 'only-on-failure',
    video: 'retain-on-failure',
  },

  projects: [
    {
      // Every API route requires authentication, so the browser suite
      // starts by signing in through the real form.
      name: 'setup',
      testMatch: /auth\.setup\.ts/,
    },
    {
      name: 'chromium',
      testDir: './specs',
      use: { browserName: 'chromium', storageState: STORAGE_STATE },
      dependencies: ['setup'],
    },
  ],

  outputDir: '../../test-results/ui',
})
