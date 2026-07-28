/**
 * Global teardown for the API E2E suite.
 *
 * Chain `afterAll` blocks handle their own cleanup. This is the backstop: it
 * removes anything this run created and a chain did not get to, whether the run
 * passed or failed. On a shared environment that matters — a task left open
 * keeps a sandbox alive.
 *
 * Only this run's prefix is swept. Older leftovers are the scheduled sweep's
 * job (`bun tests/sweep.ts`), deliberately separate: a teardown must never
 * delete something a concurrent run is still using.
 */

import { createDriver } from '../shared/moat'
import { MoatApi } from '../shared/moat-api'
import { runPrefix } from '../shared/run-scope'
import { sweep } from '../shared/sweep'

export default async function globalTeardown(): Promise<void> {
  try {
    const api = new MoatApi(await createDriver())
    await sweep(api, { onlyThisRun: true })
  } catch (error) {
    // A teardown failure must not turn a passing run red — but it must be
    // visible, because it means something may still be running.
    console.warn(
      `[global-teardown] Could not sweep ${runPrefix()}: ${
        error instanceof Error ? error.message : String(error)
      }`,
    )
  }

  console.log('\n[global-teardown] API E2E tests complete.\n')
}
