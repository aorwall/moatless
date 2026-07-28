/**
 * Multi-user fixtures for access control tests.
 *
 * Each user is created through the admin API and given an API key of their own,
 * so a request from `alice` is authenticated as alice and nothing else — there
 * is no header-trust path.
 *
 * Only a profile that can seed users has these fixtures. Elsewhere the deployed
 * auth provider owns identity, so a chain that needs a second person skips with
 * the capability named — `requiresSeededUsers()` is how it says so.
 *
 * The fixtures are worker-scoped: creating a user and minting a key costs a
 * couple of round trips, so paying it once per worker rather than once per test
 * keeps the suite fast.
 *
 * Usage:
 *   import { test, expect, requiresSeededUsers } from "../fixtures/auth-fixtures";
 *
 *   test.describe("Chain: ...", () => {
 *     requiresSeededUsers();
 *
 *     test("my test", async ({ alice }) => {
 *       const team = await alice.createTeam({ name: "Eng", slug: "eng" });
 *     });
 *   });
 */

import { test as base } from '@playwright/test'
import { type Moat, mintApiKey } from '../../shared/moat'
import { MoatApi } from '../../shared/moat-api'
import { missingCapability, profile } from '../../shared/profile'
import { test as apiTest } from './api-fixtures'

/** Password every fixture user is created with. */
export const TEST_PASSWORD = 'integration-test-password'

/**
 * Skip the enclosing describe unless the profile can create users.
 *
 * Call it at describe scope. The reason names the capability, so a skipped run
 * reads as "this environment cannot do that" rather than as a mystery.
 */
export function requiresSeededUsers(): void {
  const reason = missingCapability('seedUsers')
  base.skip(reason !== null, reason ?? '')
}

/**
 * Create `login` (idempotent) and return an API authenticated as them.
 *
 * Exported so tests that mutate team membership can re-authenticate afterwards:
 * a key carries the group it was minted with, so one minted before
 * `addTeamMember` does not see the new team.
 */
export async function loginAs(
  driver: Moat,
  login: string,
  role: 'user' | 'admin' = 'user',
): Promise<MoatApi> {
  await new MoatApi(driver).ensureUser(login, TEST_PASSWORD, role)
  const key = await mintApiKey(profile().baseUrl, login, TEST_PASSWORD)
  return new MoatApi(driver.withApiKey(key))
}

/** No test-scoped fixtures — alice/bob/carol are all worker-scoped. */
// biome-ignore lint/complexity/noBannedTypes: Playwright's extend needs an empty object type here
type NoTestFixtures = {}

export const test = apiTest.extend<
  NoTestFixtures,
  { alice: MoatApi; bob: MoatApi; carol: MoatApi }
>({
  alice: [
    async ({ driver }, use) => {
      await use(await loginAs(driver, 'alice'))
    },
    { scope: 'worker' },
  ],
  bob: [
    async ({ driver }, use) => {
      await use(await loginAs(driver, 'bob'))
    },
    { scope: 'worker' },
  ],
  carol: [
    async ({ driver }, use) => {
      await use(await loginAs(driver, 'carol', 'admin'))
    },
    { scope: 'worker' },
  ],
})

export { expect } from '@playwright/test'
