/**
 * Multi-user test fixtures for access control tests.
 *
 * Extends the base api-fixtures to provide pre-configured user clients
 * that send ForwardAuth headers to simulate different users.
 *
 * Usage:
 *   import { test, expect } from "../fixtures/auth-fixtures";
 *
 *   test("my test", async ({ alice, bob, carol }) => {
 *     const team = await alice.createTeam({ name: "Eng", slug: "eng" });
 *   });
 */

import { test as apiTest } from "./api-fixtures";
import type { ApiClient } from "../../shared/api-client";

export const test = apiTest.extend<{
	alice: ApiClient;
	bob: ApiClient;
	carol: ApiClient;
}>({
	alice: async ({ api }, use) => {
		await use(api.asUser({ userId: "alice-test", login: "alice" }));
	},
	bob: async ({ api }, use) => {
		await use(api.asUser({ userId: "bob-test", login: "bob" }));
	},
	carol: async ({ api }, use) => {
		await use(
			api.asUser({
				userId: "carol-admin",
				login: "carol",
				role: "admin",
			}),
		);
	},
});

export { expect } from "@playwright/test";
