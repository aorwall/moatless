import { resolve } from 'node:path'
import { defineConfig } from '@playwright/test'
import { describeProfile, profile } from '../shared/profile'
import { runPrefix } from '../shared/run-scope'

// A local run keeps its settings in the root .env; a sandbox run already has
// them in its environment, so a missing file is not an error.
try {
  process.loadEnvFile(resolve(__dirname, '../../.env'))
} catch {
  // No .env — the environment is expected to carry the settings already.
}

const active = profile()

/**
 * Playwright configuration for the API E2E suite.
 *
 * No browser: every call goes through the compiled `moat` binary, so the tests
 * exercise the client people actually run.
 *
 * Which environment is under test comes from the profile — `E2E_PROFILE`, or
 * inferred from whether the process carries a sandbox login. Nothing here reads
 * a URL of its own.
 *
 * Prerequisites (docker profile):
 *   Start docker-compose: cd docker-compose && ./dev-docker.sh
 *   Run: GITHUB_PAT=ghp_xxx npx playwright test --config tests/api/playwright.config.ts
 *
 * Prerequisites (k8s profile):
 *   Run from inside a sandbox, where `moat` is already logged in:
 *   E2E_PROFILE=k8s npx playwright test --config tests/api/playwright.config.ts
 */
export default defineConfig({
  testDir: './specs',

  // Global setup: check what the profile needs, and reset the database where
  // the profile says that is allowed.
  globalSetup: resolve(__dirname, './global-setup.ts'),
  globalTeardown: resolve(__dirname, './global-teardown.ts'),

  // Chains are serial internally; this allows separate chains to parallelize
  fullyParallel: false,

  // Workers: 1 on CI (safer), 3 locally (chains are independent). E2E_WORKERS
  // overrides both — chains carry a run-scope prefix and share no fixtures, so
  // what caps this is how many sandboxes the environment will hold, and only
  // whoever started it knows that.
  workers: Number(process.env.E2E_WORKERS) || (process.env.CI ? 1 : 3),

  // Fail the build on CI if you accidentally left test.only in source code
  forbidOnly: !!process.env.CI,

  // No retries — each chain should be deterministic
  retries: 0,

  // Pods schedule, pull and mount before anything starts, so the ceiling
  // tracks the profile rather than a constant.
  timeout: Math.max(active.timeouts.agentIdle + 120_000, 300_000),

  // 30 second assertion timeout
  expect: {
    timeout: 30_000,
  },

  // Reporter configuration. The metadata puts the environment under test — and
  // which capabilities it lacks — at the top of every report, so a run full of
  // skips explains itself. The prefix goes beside it because on a shared
  // environment it is the only way to tell which resources a report is about.
  //
  // This is where the prefix is minted: the config loads before global setup
  // does, in the same process, so both read the one value.
  metadata: { profile: describeProfile(active), runPrefix: runPrefix() },
  reporter: [
    ['list'],
    [resolve(__dirname, './reporters/run-summary.ts')],
    // `open: never` — by default a failing run serves the HTML report and
    // blocks until somebody closes it. Nobody is there: this suite is run
    // from a terminal that wants its prompt back and by a scheduled Loop
    // that would otherwise hang after the one outcome worth reporting.
    ['html', { outputFolder: '../../test-results/api', open: 'never' }],
    ['json', { outputFile: '../../test-results/api/results.json' }],
  ],

  use: {
    baseURL: active.baseUrl,

    // No browser features
    trace: 'off',
    screenshot: 'off',
    video: 'off',
  },

  // Single project — no browser projects
  projects: [{ name: 'api' }],

  // Output directory for test artifacts
  outputDir: '../../test-results/api',
})
