/**
 * Chain: Auth Verify (03-login-session-scenarios.md)
 *
 * Tests the /api/v1/auth/verify endpoint behavior.
 *
 * In Docker Compose dev mode (AUTH_PROVIDER unset), the verify endpoint
 * returns 200 for all requests regardless of headers. These tests validate
 * that the endpoint is accessible and responds correctly in dev mode.
 *
 * Preview access control scenarios (AUTH-33 to AUTH-35) require a real auth
 * provider and Traefik ForwardAuth setup — they cannot be tested in Docker
 * Compose dev mode. The scope-based filtering they rely on is already
 * validated by auth-access-control.spec.ts (AUTH-20 through AUTH-26).
 */

import { test, expect } from "../fixtures/auth-fixtures";

// =============================================================================
// Chain 1: ForwardAuth Verification (dev mode)
// =============================================================================

test.describe("Chain 1: ForwardAuth Verification (dev mode)", () => {
	// AUTH-30: In dev mode, even unauthenticated requests pass (no auth provider)
	test("AUTH-30: Verify endpoint returns 200 without credentials (dev mode)", async ({
		api,
	}) => {
		// Plain client with no user headers
		const response = await api.verifyAuth();
		expect(response.status).toBe(200);
	});

	// AUTH-31: Authenticated user headers pass verification
	test("AUTH-31: Verify endpoint returns 200 with user headers", async ({
		alice,
	}) => {
		const response = await alice.verifyAuth();
		expect(response.status).toBe(200);
	});

	// AUTH-32: API key header also passes (dev mode allows all)
	test("AUTH-32: Verify endpoint returns 200 with API key header", async ({
		api,
	}) => {
		const client = api.asUser({ userId: "api-key-user", login: "bot" });
		const response = await client.verifyAuth();
		expect(response.status).toBe(200);
	});
});

// =============================================================================
// Preview Access Control — documented limitations
// =============================================================================

test.describe("Chain 2: Preview Access Control (dev mode)", () => {
	// In dev mode the verify handler short-circuits before checking
	// X-Forwarded-Host, so preview access control is not exercised.
	// These tests confirm the endpoint still returns 200 with a
	// forwarded host header (no 500 / crash).

	test("AUTH-33: Verify with X-Forwarded-Host returns 200 (dev mode)", async ({
		alice,
	}) => {
		const response = await alice.verifyAuth({
			"X-Forwarded-Host": "some-task--3000.preview.example.com",
		});
		expect(response.status).toBe(200);
	});
});
