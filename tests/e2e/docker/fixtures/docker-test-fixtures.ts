/**
 * Playwright Test Fixtures for Docker E2E Tests
 *
 * Provides reusable fixtures for:
 * - API client (connected to docker-compose backend)
 * - SSE event collector
 * - Test repository (auto-created and cleaned up)
 * - Test task (auto-created and cleaned up)
 *
 * Usage:
 *   import { test, expect } from "../fixtures/docker-test-fixtures";
 *
 *   test("my test", async ({ api, sse, testTask }) => {
 *     // api, sse, and testTask are automatically available
 *   });
 */

import { test as base, expect } from "@playwright/test";
import {
	ApiClient,
	SSEEventCollector,
	getContainerNameForTask,
	isDockerComposeHealthy,
	saveContainerLogs,
} from "../helpers";
import type { Repository, Task } from "../helpers";

// =============================================================================
// Fixture Types
// =============================================================================

/**
 * Docker test fixtures available in test functions
 */
interface DockerTestFixtures {
	/** API client connected to docker-compose backend */
	api: ApiClient;

	/** SSE event collector for verifying events */
	sse: SSEEventCollector;

	/** Template repository (uses existing template from docker-compose) */
	templateRepository: {
		id: string;
		name: string;
		repository: Repository;
	};

	/** Test task (auto-created from template, automatically cleaned up) */
	testTask: {
		taskId: string;
		task: Task;
		containerName: string;
		cleanup: () => Promise<void>;
	};
}

// =============================================================================
// Base URL Configuration
// =============================================================================

function getBaseURL(): string {
	return process.env.DOCKER_COMPOSE_URL || "http://localhost:8000";
}

function getSSEStreamURL(): string {
	return `${getBaseURL()}/api/v1/events/stream`;
}

// =============================================================================
// Test Extension
// =============================================================================

export const test = base.extend<DockerTestFixtures>({
	/**
	 * API Client Fixture
	 *
	 * Provides a type-safe API client connected to the docker-compose backend.
	 * Validates that the backend is healthy before providing the client.
	 */
	api: async ({ request }, use, testInfo) => {
		const baseURL = getBaseURL();
		const api = new ApiClient(request, baseURL);

		// Verify backend is healthy
		const health = await isDockerComposeHealthy(baseURL);
		if (!health.healthy) {
			testInfo.annotations.push({
				type: "skip",
				description: `Docker-compose not healthy: ${health.error}`,
			});
			throw new Error(
				`Docker-compose environment not healthy: ${health.error}\n` +
					"Make sure to start the environment with: cd docker-compose && ./dev-docker.sh",
			);
		}

		await use(api);
	},

	/**
	 * SSE Event Collector Fixture
	 *
	 * Provides an SSE event collector that automatically:
	 * - Connects to the event stream before tests
	 * - Stops collection after tests
	 */
	sse: async ({}, use) => {
		const collector = new SSEEventCollector({ debug: !!process.env.DEBUG_SSE });

		await collector.start(getSSEStreamURL());

		// Check for connection errors
		const error = collector.getError();
		if (error) {
			throw new Error(`Failed to connect to SSE stream: ${error.message}`);
		}

		await use(collector);

		collector.stop();
	},

	/**
	 * Template Repository Fixture
	 *
	 * Uses the existing template repository from docker-compose environment.
	 * This avoids needing GitHub authentication for cloning.
	 */
	templateRepository: async ({ api }, use, testInfo) => {
		// Get the first template repository from the docker-compose environment
		const repository = await api.getTemplateRepository();

		if (!repository) {
			throw new Error(
				"No template repository found in docker-compose environment.\n" +
					"Make sure the docker-compose environment is properly initialized.",
			);
		}

		// Attach repository ID to test info for debugging
		testInfo.annotations.push({
			type: "repository",
			description: repository.id,
		});

		await use({
			id: repository.id,
			name: repository.name,
			repository,
		});

		// No cleanup needed - we don't own this repository
	},

	/**
	 * Test Task Fixture
	 *
	 * Creates a vibe task from the template repository. The task is created
	 * WITHOUT a message to prevent auto-starting the agent. Automatically cleaned up.
	 *
	 * Depends on templateRepository fixture.
	 */
	testTask: async ({ api, templateRepository }, use, testInfo) => {
		// Create vibe task from template with start: false to prevent auto-starting the agent
		// Note: vibe tasks require a message, but we set start: false to control when the agent runs
		const task = await api.createVibeTask({
			agentType: "claude-code",
			templateId: templateRepository.id,
			message: "E2E test task - do not execute automatically",
			start: false, // Don't auto-start the agent
		});

		const containerName = getContainerNameForTask(task.taskId);

		const cleanup = async () => {
			// Save container logs on failure for debugging
			if (testInfo.status === "failed" || testInfo.status === "timedOut") {
				try {
					const logPath = `/tmp/e2e-docker-${task.taskId.slice(0, 8)}-${Date.now()}.log`;
					await saveContainerLogs(containerName, logPath, {
						testName: testInfo.title,
					});
					testInfo.attachments.push({
						name: "container-logs",
						path: logPath,
						contentType: "text/plain",
					});
				} catch {
					// Log capture failure shouldn't break cleanup
				}
			}

			// Stop agent if running
			try {
				await api.stopTask(task.taskId);
				// Brief wait for cleanup
				await new Promise((resolve) => setTimeout(resolve, 500));
			} catch {
				// Agent might not be running
			}

			// Delete task (triggers container cleanup)
			try {
				await api.deleteTask(task.taskId);
			} catch {
				// Task might already be deleted
			}
		};

		// Attach task ID to test info for debugging
		testInfo.annotations.push({
			type: "task",
			description: task.taskId,
		});
		testInfo.annotations.push({
			type: "container",
			description: containerName,
		});

		await use({
			taskId: task.taskId,
			task,
			containerName,
			cleanup,
		});

		// Cleanup after test
		await cleanup();
	},
});

// =============================================================================
// Re-export expect
// =============================================================================

export { expect };
