/**
 * Chain: Auth Verify (03-login-session-scenarios.md)
 *
 * Tests the /api/v1/auth/verify endpoint behavior.
 *
 * Preview access control (AUTH-33 to AUTH-35) is exercised here for the first
 * time: `verify_handler` no longer short-circuits, so the task lookup behind a
 * preview hostname actually runs.
 */

import { ApiClient } from "../../shared/api-client";
import { getBaseURL } from "../fixtures/api-fixtures";
import { expect, test } from "../fixtures/auth-fixtures";

// =============================================================================
// Chain 1: ForwardAuth Verification
// =============================================================================

test.describe("Chain 1: ForwardAuth Verification", () => {
	// AUTH-30: an unauthenticated request is rejected
	test("AUTH-30: Verify endpoint returns 401 without credentials", async () => {
		const response = await new ApiClient(getBaseURL()).verifyAuth();
		expect(response.status).toBe(401);
	});

	// AUTH-30.1: forged X-User-* headers are not an identity — the auth
	// middleware strips them before validating.
	test("AUTH-30.1: Verify endpoint returns 401 for forged user headers", async () => {
		const response = await new ApiClient(getBaseURL()).verifyAuth({
			"X-User-Id": "admin",
			"X-User-Role": "admin",
		});
		expect(response.status).toBe(401);
	});

	// AUTH-31: a real session cookie passes verification
	test("AUTH-31: Verify endpoint returns 200 with a session cookie", async ({
		alice,
	}) => {
		const response = await alice.verifyAuth();
		expect(response.status).toBe(200);
	});

	// AUTH-32: a garbage cookie is rejected
	test("AUTH-32: Verify endpoint returns 401 for an invalid cookie", async () => {
		const client = new ApiClient(getBaseURL()).withSessionCookie(
			"moatless_session=not-a-valid-jwt",
		);
		const response = await client.verifyAuth();
		expect(response.status).toBe(401);
	});
});

// =============================================================================
// Chain 2: Preview Access Control
// =============================================================================

test.describe("Chain 2: Preview Access Control", () => {
	// AUTH-35: a preview host for a task that does not exist is denied
	test("AUTH-35: Verify with an unknown task's preview host returns 403", async ({
		alice,
	}) => {
		const response = await alice.verifyAuth({
			"X-Forwarded-Host": "no-such-task--3000.preview.example.com",
		});
		expect(response.status).toBe(403);
	});

	// AUTH-33 / AUTH-34: an admin reaches any preview host; an unrelated user
	// does not.
	test("AUTH-33/34: Preview access is scoped to the task", async ({
		api,
		alice,
	}) => {
		const repos = await api.listRepositories();
		const repoId = repos[0]?.id;
		test.skip(!repoId, "No repository available to create a task under");

		const task = await api.createVibeTask({
			templateId: repoId as string,
			message: "Preview access control task",
		});
		const host = `${task.taskId}--3000.preview.example.com`;

		const adminResponse = await api.verifyAuth({ "X-Forwarded-Host": host });
		expect(adminResponse.status).toBe(200);

		const strangerResponse = await alice.verifyAuth({
			"X-Forwarded-Host": host,
		});
		expect(strangerResponse.status).toBe(403);
	});
});
