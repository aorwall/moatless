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
		const api = new ApiClient(getBaseURL());

		const healthy = await api.isHealthy();
		if (!healthy) {
			throw new Error(
				"Backend not healthy. Make sure to start the environment with: cd docker-compose && ./dev-docker.sh",
			);
		}

		await use(api);
	},

	/**
	 * SSE Event Collector Fixture
	 *
	 * Automatically connects to the NDJSON event stream before each test
	 * and stops collection after the test completes.
	 */
	sse: async ({}, use) => {
		const collector = new SSEEventCollector({ debug: !!process.env.DEBUG_SSE });
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
