/**
 * Playwright Test Fixtures for API-only E2E Tests.
 *
 * Provides reusable fixtures:
 * - api: Fetch-based API client connected to the backend
 * - sse: SSE NDJSON event collector
 *
 * Usage:
 *   import { test, expect } from "../fixtures/api-fixtures";
 *
 *   test("my test", async ({ api, sse }) => {
 *     const task = await api.createVibeTask({ ... });
 *   });
 */

import { test as base, expect } from "@playwright/test";
import { ApiClient } from "../../shared/api-client";
import { SSEEventCollector } from "../../shared/sse-collector";

// =============================================================================
// Fixture Types
// =============================================================================

interface TestFixtures {
	/** Fetch-based API client connected to the backend */
	api: ApiClient;

	/** SSE NDJSON event collector (auto-connects, auto-stops) */
	sse: SSEEventCollector;
}

// =============================================================================
// Configuration
// =============================================================================

export function getBaseURL(): string {
	return process.env.DOCKER_COMPOSE_URL || `http://localhost:${process.env.BACKEND_PORT || "8080"}`;
}

/** Bootstrap admin credentials, matching docker-compose.ci.yml. */
export function getAdminCredentials(): { username: string; password: string } {
	return {
		username: process.env.ADMIN_USERNAME || "admin",
		password: process.env.ADMIN_PASSWORD || "ci-admin-password-1234",
	};
}

/**
 * The admin session cookie, resolved once per worker.
 *
 * argon2 verification costs ~100ms, so this is cached rather than re-logged-in
 * for every test.
 */
let adminCookiePromise: Promise<string> | null = null;

export function getAdminCookie(baseURL: string): Promise<string> {
	if (!adminCookiePromise) {
		const { username, password } = getAdminCredentials();
		adminCookiePromise = new ApiClient(baseURL).loginCookie(username, password);
	}
	return adminCookiePromise;
}

// =============================================================================
// Test Extension
// =============================================================================

export const test = base.extend<TestFixtures>({
	/**
	 * API Client Fixture
	 *
	 * Provides a type-safe API client connected to the backend.
	 * Validates that the backend is healthy before providing the client.
	 */
	api: async ({}, use) => {
		const anonymous = new ApiClient(getBaseURL());

		const healthy = await anonymous.isHealthy();
		if (!healthy) {
			throw new Error(
				"Backend not healthy. Make sure to start the environment with: cd docker-compose && ./dev-docker.sh",
			);
		}

		// Every API route requires authentication; the default client is admin.
		await use(anonymous.withSessionCookie(await getAdminCookie(getBaseURL())));
	},

	/**
	 * SSE Event Collector Fixture
	 *
	 * Automatically connects to the NDJSON event stream before each test
	 * and stops collection after the test completes.
	 */
	sse: async ({}, use) => {
		const collector = new SSEEventCollector({
			debug: !!process.env.DEBUG_SSE,
			headers: { Cookie: await getAdminCookie(getBaseURL()) },
		});
		await collector.start(`${getBaseURL()}/api/v1/events/stream`);

		const error = collector.getError();
		if (error) {
			throw new Error(`Failed to connect to SSE stream: ${error.message}`);
		}

		await use(collector);

		collector.stop();
	},
});

export { expect };
