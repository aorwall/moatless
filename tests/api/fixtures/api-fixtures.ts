/**
 * Fixtures for the API suite.
 *
 * Two fixtures, and both come from the profile rather than from the
 * environment directly:
 *
 * - `api` — domain verbs, spoken through the compiled `moat` binary.
 * - `events` — a live event stream, collected from the CLI.
 *
 * Which environment those talk to, and what it is capable of, is `profile.ts`'s
 * business. Nothing here reads a URL or a credential of its own.
 */

import { test as base } from '@playwright/test'
import { EventStream } from '../../shared/event-stream'
import { createDriver, type Moat } from '../../shared/moat'
import { MoatApi } from '../../shared/moat-api'
import { describeProfile, profile } from '../../shared/profile'

interface TestFixtures {
  /** Domain verbs against the environment under test. */
  api: MoatApi
  /** Live events, started before the test and stopped after it. */
  events: EventStream
}

interface WorkerFixtures {
  /** The driver, built once per worker — a login costs a round trip. */
  driver: Moat
}

export const test = base.extend<TestFixtures, WorkerFixtures>({
  driver: [
    async ({}, use) => {
      const driver = await createDriver()
      await use(driver)
    },
    { scope: 'worker' },
  ],

  api: async ({ driver }, use) => {
    const api = new MoatApi(driver)
    if (!(await api.isHealthy())) {
      throw new Error(`Backend is not healthy at ${profile().baseUrl} (${describeProfile()}).`)
    }
    await use(api)
  },

  events: async ({ driver }, use) => {
    const stream = new EventStream(driver)
    await use(stream)
    stream.stop()
  },
})

// A spec imports its whole vocabulary from here.
export { expect } from '@playwright/test'
export { describeProfile, profile } from '../../shared/profile'
