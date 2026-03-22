/**
 * Shared fixtures for UI E2E tests.
 *
 * Provides:
 * - `api`: ApiClient instance connected to the running backend
 * - `testRepo`: Ensures a template repository exists for task creation
 */

import { test as base } from "@playwright/test";
import { ApiClient } from "../../shared/api-client";

function getBaseURL(): string {
	return (
		process.env.DOCKER_COMPOSE_URL ||
		`http://localhost:${process.env.BACKEND_PORT || "8080"}`
	);
}

export interface UITestFixtures {
	/** Type-safe API client for setup/teardown operations. */
	api: ApiClient;
	/** Template repository ID, created if missing. */
	testRepoId: string;
}

export const test = base.extend<UITestFixtures>({
	api: async ({}, use) => {
		const api = new ApiClient(getBaseURL());
		const healthy = await api.isHealthy();
		if (!healthy) {
			throw new Error(
				`Backend not healthy at ${getBaseURL()} — is docker-compose running?`,
			);
		}
		await use(api);
	},

	testRepoId: async ({ api }, use) => {
		const repo = await api.ensureTemplateRepository();
		await use(repo.id);
	},
});

export { expect } from "@playwright/test";
