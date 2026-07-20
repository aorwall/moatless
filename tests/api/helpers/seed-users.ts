/**
 * Creates the standard test users through the admin API and logs them in.
 *
 * Users are real accounts with real passwords — there is no header-trust path,
 * so a test client is a session cookie exactly like a browser's.
 */

import { ApiClient } from "../../shared/api-client";
import {
	getAdminCookie,
	getBaseURL,
} from "../fixtures/api-fixtures";

/** Password every seeded user is created with. */
export const TEST_PASSWORD = "integration-test-password";

export const ALICE = { login: "alice", role: "user" } as const;
export const BOB = { login: "bob", role: "user" } as const;
export const CAROL_ADMIN = { login: "carol", role: "admin" } as const;

/**
 * Create alice, bob and carol if they do not already exist. Idempotent — an
 * existing login answers 409, which `ensureUser` treats as success.
 */
export async function seedTestUsers(): Promise<void> {
	const baseURL = getBaseURL();
	const admin = new ApiClient(baseURL).withSessionCookie(
		await getAdminCookie(baseURL),
	);

	for (const user of [ALICE, BOB, CAROL_ADMIN]) {
		await admin.ensureUser(user.login, TEST_PASSWORD, user.role);
	}
}
