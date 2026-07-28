/**
 * Global setup for the API E2E suite.
 *
 * Runs once before all test files:
 * 1. Announces the environment under test and what it cannot do
 * 2. Checks the fixture the profile names is actually there
 * 3. Resets the database, where the profile says that is allowed
 *
 * Nothing here fails a run for a missing optional credential. A test that needs
 * one skips with the reason named, so a partial environment produces a partial
 * run rather than no run at all. A missing *fixture* is different: it is not
 * optional, and every chain would fail on it in turn.
 */

import { createDriver } from '../shared/moat'
import { MoatApi } from '../shared/moat-api'
import { describeProfile, profile } from '../shared/profile'
import { runPrefix } from '../shared/run-scope'
import { resetDatabase } from './helpers/db-reset'

export default async function globalSetup(): Promise<void> {
  const active = profile()

  // Mint the run prefix here and publish it, so every worker — and the
  // teardown, which is a separate process — names the same run. Workers
  // inherit this environment when Playwright forks them, which happens after
  // global setup returns.
  process.env.E2E_RUN_PREFIX = runPrefix()

  console.log(`\n[global-setup] Testing ${describeProfile(active)}`)
  console.log(`[global-setup] Run prefix: ${process.env.E2E_RUN_PREFIX}\n`)

  // A profile that names a fixture by id is pointing at something the run does
  // not create. Checking it once here turns "every chain failed" into one line
  // naming the identifier and the variable that overrides it.
  if (active.fixture.repositoryId) {
    const api = new MoatApi(await createDriver(active))
    const repo = await api.ensureTemplateRepository()
    console.log(`[global-setup] Fixture repository: ${repo.name} (${repo.id})`)
  }

  if (active.capabilities.resetDatabase) {
    await resetDatabase()
  } else {
    console.log(
      `[global-setup] Profile '${active.name}' shares its database with real work — not resetting it.`,
    )
  }

  console.log('[global-setup] Setup complete.\n')
}
