/**
 * Multi-user test fixtures for access control tests.
 *
 * Each user is created through the admin API and logged in with a real
 * password, exactly as a browser would — there is no header-trust path.
 *
 * The fixtures are worker-scoped: argon2 verification costs ~100ms per login,
 * so paying it once per worker rather than once per test keeps the suite fast.
 *
 * Usage:
 *   import { test, expect } from "../fixtures/auth-fixtures";
 *
 *   test("my test", async ({ alice, bob, carol }) => {
 *     const team = await alice.createTeam({ name: "Eng", slug: "eng" });
 *   });
 */

import type { ApiClient } from "../../shared/api-client";
import { getAdminCookie, getBaseURL, test as apiTest } from "./api-fixtures";
import { ApiClient as ApiClientImpl } from "../../shared/api-client";

/** Password every fixture user is created with. */
export const TEST_PASSWORD = "integration-test-password";

/**
 * Create `login` (idempotent) and return a client authenticated as them.
 *
 * Exported so tests that mutate team membership can re-login afterwards: team
 * ids are baked into the session JWT at login and never re-resolved per
 * request, so a client minted before `addTeamMember` carries no teams.
 */
export async function loginAs(
	login: string,
	role: "user" | "admin" = "user",
): Promise<ApiClient> {
	const baseURL = getBaseURL();
	const admin = new ApiClientImpl(baseURL).withSessionCookie(
		await getAdminCookie(baseURL),
	);
	await admin.ensureUser(login, TEST_PASSWORD, role);
	return new ApiClientImpl(baseURL).asUser(login, TEST_PASSWORD);
}

/** No test-scoped fixtures — alice/bob/carol are all worker-scoped. */
interface NoTestFixtures {}

export const test = apiTest.extend<
	NoTestFixtures,
	{ alice: ApiClient; bob: ApiClient; carol: ApiClient }
>({
	alice: [
		async ({}, use) => {
			await use(await loginAs("alice"));
		},
		{ scope: "worker" },
	],
	bob: [
		async ({}, use) => {
			await use(await loginAs("bob"));
		},
		{ scope: "worker" },
	],
	carol: [
		async ({}, use) => {
			await use(await loginAs("carol", "admin"));
		},
		{ scope: "worker" },
	],
});

export { expect } from "@playwright/test";
