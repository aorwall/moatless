import { resolve } from 'node:path'
import { defineConfig, devices } from '@playwright/test'
import { storageStatePath, uiEnv } from './helpers/env'

try {
  process.loadEnvFile(resolve(__dirname, '../../.env'))
} catch {
  // No .env — the environment is expected to carry the settings already.
}

const env = uiEnv()

/**
 * Browser suite for the T3 client against a deployed Moatless backend. Unlike
 * `tests/api`, it drives the real UI and spends real agent turns, so it runs
 * against a deployment by hand rather than in the local harness.
 *
 *   E2E_UI_URL=https://moatless.example.com MOATLESS_API_KEY=mvk_… \
 *     bunx playwright test --config tests/ui/playwright.config.ts
 */
export default defineConfig({
  testDir: './specs',
  globalSetup: resolve(__dirname, './global-setup.ts'),
  fullyParallel: false,
  workers: 1,
  forbidOnly: !!process.env.CI,
  retries: 0,
  // Sandbox provisioning plus one agent turn; each step sets its own budget.
  timeout: 15 * 60_000,
  expect: { timeout: 30_000 },
  metadata: { target: env.baseUrl, project: env.project },
  reporter: [['list'], ['html', { outputFolder: '../../test-results/ui/report', open: 'never' }]],
  use: {
    ...devices['Desktop Chrome'],
    viewport: { width: 1440, height: 900 },
    baseURL: env.baseUrl,
    storageState: storageStatePath,
    trace: 'retain-on-failure',
    screenshot: 'only-on-failure',
    video: 'off',
  },
  projects: [{ name: 'chromium' }],
  outputDir: '../../test-results/ui/artifacts',
})
