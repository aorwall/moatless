/**
 * Playwright Test Fixtures for Docker E2E Tests
 *
 * Provides reusable fixtures for:
 * - API client (connected to docker-compose backend)
 * - SSE event collector
 * - Test repository (uses existing template from docker-compose)
 * - Test task (WORKER-SCOPED: shared across all tests for performance)
 *
 * Worker-scoped fixtures (shared across all tests in a worker):
 * - sharedTaskState: The actual task/container created once per worker
 *
 * Test-scoped fixtures:
 * - api: Playwright's request-based API client
 * - sse: SSE event collector
 * - testTask: Exposes sharedTaskState to tests
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
 * Worker-scoped fixtures (shared across all tests in a worker)
 */
interface WorkerFixtures {
	/** Shared task state created once per worker */
	sharedTaskState: {
		taskId: string;
		task: Task;
		containerName: string;
		templateRepository: Repository;
	};
}

/**
 * Test-scoped fixtures available in test functions
 */
interface TestFixtures {
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

	/** Test task (references worker-scoped shared task) */
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
// Fetch-based API helpers for worker-scoped fixtures
// (Worker fixtures can't use Playwright's test-scoped `request` fixture)
// =============================================================================

async function fetchJson<T>(url: string, options?: RequestInit): Promise<T> {
	const response = await fetch(url, {
		...options,
		headers: {
			"Content-Type": "application/json",
			...options?.headers,
		},
	});
	if (!response.ok) {
		const text = await response.text();
		throw new Error(`HTTP ${response.status}: ${text}`);
	}
	return response.json();
}

async function getTemplateRepositoryViaFetch(baseURL: string): Promise<Repository | null> {
	const data = await fetchJson<{ repositories: Repository[] }>(`${baseURL}/api/v1/repositories`);
	return data.repositories.find((r) => r.kind === "template") || null;
}

async function createVibeTaskViaFetch(
	baseURL: string,
	config: { agentType: string; templateId: string; message?: string; start?: boolean },
): Promise<Task> {
	return fetchJson<Task>(`${baseURL}/api/v1/tasks/vibe`, {
		method: "POST",
		body: JSON.stringify(config),
	});
}

async function stopTaskViaFetch(baseURL: string, taskId: string): Promise<void> {
	try {
		await fetch(`${baseURL}/api/v1/tasks/${taskId}/stop`, { method: "POST" });
	} catch {
		// Ignore errors - task may not be running
	}
}

async function deleteTaskViaFetch(baseURL: string, taskId: string): Promise<void> {
	try {
		await fetch(`${baseURL}/api/v1/tasks/${taskId}`, { method: "DELETE" });
	} catch {
		// Ignore errors - task may already be deleted
	}
}

// =============================================================================
// Test Extension
// =============================================================================

export const test = base.extend<TestFixtures, WorkerFixtures>({
	/**
	 * WORKER-SCOPED: Shared Task State
	 *
	 * Creates ONE task per worker, shared across ALL tests.
	 * This dramatically reduces test time by avoiding container spin-up for each test.
	 *
	 * Uses fetch-based API since worker fixtures can't depend on test-scoped `request`.
	 */
	sharedTaskState: [
		async ({}, use, workerInfo) => {
			const baseURL = getBaseURL();

			// Verify backend is healthy
			const health = await isDockerComposeHealthy(baseURL);
			if (!health.healthy) {
				throw new Error(
					`Docker-compose environment not healthy: ${health.error}\n` +
						"Make sure to start the environment with: cd docker-compose && ./dev-docker.sh",
				);
			}

			// Get template repository
			const templateRepository = await getTemplateRepositoryViaFetch(baseURL);
			if (!templateRepository) {
				throw new Error(
					"No template repository found in docker-compose environment.\n" +
						"Make sure the docker-compose environment is properly initialized.",
				);
			}

			// Create vibe task from template - this task is shared across ALL tests
			const task = await createVibeTaskViaFetch(baseURL, {
				agentType: "claude-code",
				templateId: templateRepository.id,
				message: "E2E test task - shared across all tests",
				start: false,
			});

			const containerName = getContainerNameForTask(task.taskId);

			console.log(
				`[Worker ${workerInfo.workerIndex}] Created shared task: ${task.taskId} (container: ${containerName})`,
			);

			await use({
				taskId: task.taskId,
				task,
				containerName,
				templateRepository,
			});

			// Cleanup runs ONCE after ALL tests in this worker complete
			console.log(`[Worker ${workerInfo.workerIndex}] Cleaning up shared task: ${task.taskId}`);

			await stopTaskViaFetch(baseURL, task.taskId);
			await new Promise((resolve) => setTimeout(resolve, 500));
			await deleteTaskViaFetch(baseURL, task.taskId);
		},
		{ scope: "worker" },
	],

	/**
	 * API Client Fixture (test-scoped)
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
	 * SSE Event Collector Fixture (test-scoped)
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
	 * Template Repository Fixture (test-scoped)
	 *
	 * Uses the existing template repository from docker-compose environment.
	 * References the repository from the worker-scoped sharedTaskState.
	 */
	templateRepository: async ({ sharedTaskState }, use, testInfo) => {
		const repository = sharedTaskState.templateRepository;

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
	},

	/**
	 * Test Task Fixture (test-scoped)
	 *
	 * Exposes the worker-scoped shared task to tests.
	 * The task is created ONCE per worker and reused across ALL tests.
	 *
	 * Note: cleanup() is a no-op since cleanup happens at worker level.
	 */
	testTask: async ({ sharedTaskState }, use, testInfo) => {
		const { taskId, task, containerName } = sharedTaskState;

		// Attach task ID to test info for debugging
		testInfo.annotations.push({
			type: "task",
			description: taskId,
		});
		testInfo.annotations.push({
			type: "container",
			description: containerName,
		});

		// Cleanup is a no-op for individual tests - cleanup happens at worker level
		const cleanup = async () => {
			// Save container logs on test failure
			if (testInfo.status === "failed" || testInfo.status === "timedOut") {
				try {
					const logPath = `/tmp/e2e-docker-${taskId.slice(0, 8)}-${testInfo.title.replace(/\s+/g, "-")}-${Date.now()}.log`;
					await saveContainerLogs(containerName, logPath, {
						testName: testInfo.title,
					});
					testInfo.attachments.push({
						name: "container-logs",
						path: logPath,
						contentType: "text/plain",
					});
				} catch {
					// Log capture failure shouldn't break anything
				}
			}
			// Note: Don't delete task - it's shared across tests
		};

		await use({
			taskId,
			task,
			containerName,
			cleanup,
		});

		// Call cleanup to capture logs on failure, but don't delete
		await cleanup();
	},
});

// =============================================================================
// Re-export expect
// =============================================================================

export { expect };
