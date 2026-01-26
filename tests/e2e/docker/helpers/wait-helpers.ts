/**
 * Wait/polling utilities for Docker E2E tests.
 *
 * Provides helper functions to wait for various async conditions
 * during E2E testing against docker-compose environment.
 */

import type { ApiClient, PreviewStatus, WorkspaceStatus } from "./api-client";
import type { SSEEventCollector } from "./sse-collector";

// =============================================================================
// Types
// =============================================================================

export interface WaitOptions {
	/** Maximum time to wait in milliseconds (default: 60000) */
	timeout?: number;
	/** Polling interval in milliseconds (default: 2000) */
	interval?: number;
	/** Optional callback for progress updates */
	onProgress?: (message: string) => void;
}

// =============================================================================
// Wait Functions
// =============================================================================

/**
 * Wait for workspace to become ready (environment provisioned)
 *
 * @param api - API client instance
 * @param taskId - Task ID to check
 * @param options - Wait options
 * @returns true if workspace is ready, false if timeout
 */
export async function waitForWorkspaceReady(
	api: ApiClient,
	taskId: string,
	options?: WaitOptions,
): Promise<boolean> {
	const { timeout = 180000, interval = 2000, onProgress } = options || {};
	const startTime = Date.now();

	while (Date.now() - startTime < timeout) {
		try {
			const status = await api.getWorkspaceStatus(taskId);

			if (status.environmentStatus === "ready" || status.workspaceReady) {
				onProgress?.("Workspace is ready");
				return true;
			}

			onProgress?.(`Workspace status: ${status.environmentStatus}`);
		} catch (err) {
			onProgress?.(`Error checking workspace: ${err}`);
		}

		await new Promise((resolve) => setTimeout(resolve, interval));
	}

	return false;
}

/**
 * Wait for preview server to be ready
 *
 * @param api - API client instance
 * @param taskId - Task ID to check
 * @param options - Wait options
 * @returns PreviewStatus if ready, null if timeout
 */
export async function waitForPreviewReady(
	api: ApiClient,
	taskId: string,
	options?: WaitOptions,
): Promise<PreviewStatus | null> {
	const { timeout = 120000, interval = 2000, onProgress } = options || {};
	const startTime = Date.now();

	while (Date.now() - startTime < timeout) {
		try {
			const status = await api.getPreviewStatus(taskId);

			if (status.status === "started") {
				onProgress?.(`Preview ready at ${status.url}`);
				return status;
			}

			if (status.status === "failed") {
				onProgress?.(`Preview failed: ${status.error}`);
				return status;
			}

			onProgress?.(`Preview status: ${status.status}`);
		} catch (err) {
			onProgress?.(`Error checking preview: ${err}`);
		}

		await new Promise((resolve) => setTimeout(resolve, interval));
	}

	return null;
}

/**
 * Wait for agent to complete execution (become idle after running)
 *
 * This function first waits for the agent to start (state becomes 'running'),
 * then waits for it to become idle (state becomes 'idle').
 * This prevents false positives when agent hasn't started yet.
 *
 * @param api - API client instance
 * @param taskId - Task ID to check
 * @param options - Wait options
 * @returns true if agent completed, false if timeout
 */
export async function waitForAgentComplete(
	api: ApiClient,
	taskId: string,
	options?: WaitOptions,
): Promise<boolean> {
	const { timeout = 300000, interval = 2000, onProgress } = options || {};
	const startTime = Date.now();
	let hasSeenRunning = false;

	while (Date.now() - startTime < timeout) {
		try {
			const status = await api.getWorkspaceStatus(taskId);
			const agentState = status.agentState;

			// Track if we've seen the agent running
			if (agentState === "running") {
				hasSeenRunning = true;
				onProgress?.("Agent is running...");
				await new Promise((resolve) => setTimeout(resolve, interval));
				continue;
			}

			// Agent is idle or none - check if it ran
			if (hasSeenRunning && (agentState === "idle" || agentState === "none")) {
				// Verify agent did some work by checking task
				const task = await api.getTask(taskId);
				if (task.numTurns > 0) {
					onProgress?.(`Agent completed with ${task.numTurns} turns`);
					return true;
				}
			}

			onProgress?.(`Agent state: ${agentState || "unknown"}`);
		} catch (err) {
			onProgress?.(`Error checking agent: ${err}`);
		}

		await new Promise((resolve) => setTimeout(resolve, interval));
	}

	return false;
}

/**
 * Wait for agent to become idle using SSE events
 *
 * Waits for either agent.completed or taskIteration.completed event.
 * Falls back to API polling if SSE events are not received.
 *
 * @param sse - SSE collector instance
 * @param api - API client instance (for fallback)
 * @param taskId - Task ID to check
 * @param options - Wait options
 * @returns true if agent completed, false if timeout
 */
export async function waitForAgentIdleViaSSE(
	sse: SSEEventCollector,
	api: ApiClient,
	taskId: string,
	options?: WaitOptions,
): Promise<boolean> {
	const { timeout = 300000, onProgress } = options || {};

	// First, try to wait for completion event via SSE
	const completionEvent = await sse.waitForAnyEvent({
		events: [
			{ type: "agent", subType: "completed" },
			{ type: "taskIteration", subType: "completed" },
		],
		taskId,
		timeout: timeout * 0.9, // Use 90% of timeout for SSE
	});

	if (completionEvent) {
		onProgress?.(`Agent completed (via SSE: ${completionEvent.type}.${completionEvent.subType})`);
		return true;
	}

	// Check for error event
	const errorEvent = await sse.waitForEvent({
		type: "agent",
		subType: "error",
		taskId,
		timeout: 2000,
	});

	if (errorEvent) {
		onProgress?.(`Agent error: ${JSON.stringify(errorEvent.payload)}`);
		return false;
	}

	// Fallback to API polling for remaining time
	onProgress?.("No SSE event, falling back to API polling...");
	const remainingTimeout = timeout * 0.1;
	return waitForAgentComplete(api, taskId, { timeout: remainingTimeout, onProgress });
}

/**
 * Wait for a specific workspace status
 *
 * @param api - API client instance
 * @param taskId - Task ID to check
 * @param expectedStatus - Expected environment status
 * @param options - Wait options
 * @returns WorkspaceStatus if matched, null if timeout
 */
export async function waitForWorkspaceStatus(
	api: ApiClient,
	taskId: string,
	expectedStatus: string,
	options?: WaitOptions,
): Promise<WorkspaceStatus | null> {
	const { timeout = 60000, interval = 2000, onProgress } = options || {};
	const startTime = Date.now();

	while (Date.now() - startTime < timeout) {
		try {
			const status = await api.getWorkspaceStatus(taskId);

			if (status.environmentStatus === expectedStatus) {
				onProgress?.(`Workspace status is ${expectedStatus}`);
				return status;
			}

			onProgress?.(`Current status: ${status.environmentStatus}, waiting for ${expectedStatus}`);
		} catch (err) {
			onProgress?.(`Error checking status: ${err}`);
		}

		await new Promise((resolve) => setTimeout(resolve, interval));
	}

	return null;
}

/**
 * Wait for a condition to be true
 *
 * Generic polling helper for custom conditions.
 *
 * @param condition - Function that returns true when condition is met
 * @param options - Wait options
 * @returns true if condition was met, false if timeout
 */
export async function waitForCondition(
	condition: () => Promise<boolean>,
	options?: WaitOptions,
): Promise<boolean> {
	const { timeout = 60000, interval = 2000, onProgress } = options || {};
	const startTime = Date.now();

	while (Date.now() - startTime < timeout) {
		try {
			if (await condition()) {
				onProgress?.("Condition met");
				return true;
			}
		} catch (err) {
			onProgress?.(`Error checking condition: ${err}`);
		}

		await new Promise((resolve) => setTimeout(resolve, interval));
	}

	return false;
}

/**
 * Wait for file to exist in workspace
 *
 * @param api - API client instance
 * @param taskId - Task ID
 * @param filePath - Path to file
 * @param options - Wait options
 * @returns true if file exists, false if timeout
 */
export async function waitForFile(
	api: ApiClient,
	taskId: string,
	filePath: string,
	options?: WaitOptions,
): Promise<boolean> {
	const { timeout = 30000, interval = 2000, onProgress } = options || {};

	return waitForCondition(
		async () => {
			try {
				await api.getFileContent(taskId, filePath);
				return true;
			} catch {
				return false;
			}
		},
		{ timeout, interval, onProgress },
	);
}
