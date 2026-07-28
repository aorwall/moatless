/**
 * Shared fixtures for UI E2E tests.
 *
 * The browser exercises the frontend; setup and teardown go through the moat
 * CLI, the same driver the API suite uses, so both suites reach the backend the
 * same way and neither carries a URL of its own.
 *
 * Provides:
 * - `api`: domain verbs against the environment under test
 * - `testRepoId`: the repository tasks are created under
 */

import { test as base } from '@playwright/test'
import { createDriver, type Moat } from '../../shared/moat'
import { MoatApi } from '../../shared/moat-api'
import { describeProfile } from '../../shared/profile'

export interface UITestFixtures {
  /** Domain verbs for setup/teardown operations. */
  api: MoatApi
  /** Template repository ID, created if missing. */
  testRepoId: string
}

export interface UIWorkerFixtures {
  /** The driver, built once per worker — a login costs a round trip. */
  driver: Moat
}

export const test = base.extend<UITestFixtures, UIWorkerFixtures>({
  driver: [
    async ({}, use) => {
      await use(await createDriver())
    },
    { scope: 'worker' },
  ],

  api: async ({ driver }, use) => {
    const api = new MoatApi(driver)
    if (!(await api.isHealthy())) {
      throw new Error(`Backend is not healthy (${describeProfile()})`)
    }
    await use(api)
  },

  testRepoId: async ({ api }, use) => {
    const repo = await api.ensureTemplateRepository()
    await use(repo.id)
  },
})

export { expect } from '@playwright/test'
