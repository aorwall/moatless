/**
 * Agent step functions (AGENT-01 through AGENT-05).
 *
 * Each function performs an action and returns a result.
 * Tests add assertions; preconditions skip them.
 */

import type {
	ApiClient,
	GetMessagesResponse,
	UiMessage,
} from "../../shared/api-client";
import type { SSEEventCollector, SSEEvent } from "../../shared/sse-collector";
import { waitForAgentComplete } from "../../shared/wait-helpers";

// =============================================================================
// AGENT-01: Wait for agent turn to complete
// =============================================================================

/**
 * Wait for the Nth taskTurn.completed event via SSE, falling back to API polling.
 */
export async function waitForAgentTurnComplete(
	sse: SSEEventCollector,
	api: ApiClient,
	taskId: string,
	turnNumber = 1,
	timeoutMs = 300000,
): Promise<{ completed: boolean; event: SSEEvent | null }> {
	const event = await sse.waitForNthEvent(
		"taskTurn",
		"completed",
		taskId,
		turnNumber,
		timeoutMs * 0.9,
	);

	if (event) {
		return { completed: true, event };
	}

	// Fallback: poll API
	const completed = await waitForAgentComplete(api, taskId, {
		timeout: timeoutMs * 0.1,
	});
	return { completed, event: null };
}

// =============================================================================
// AGENT-02: Send follow-up message
// =============================================================================

export async function sendFollowUpMessage(
	api: ApiClient,
	taskId: string,
	message: string,
): Promise<void> {
	await api.sendMessage(taskId, message);
}

// =============================================================================
// AGENT-04: Stop agent mid-execution
// =============================================================================

export async function stopAgent(
	api: ApiClient,
	taskId: string,
): Promise<void> {
	await api.stopTask(taskId);
}

// =============================================================================
// AGENT-01.3 / AGENT-05.1: Get task messages
// =============================================================================

export async function getTaskMessages(
	api: ApiClient,
	taskId: string,
): Promise<GetMessagesResponse> {
	return api.getTaskMessages(taskId);
}

// =============================================================================
// MSG helpers: Queued message filtering
// =============================================================================

/**
 * Return user messages that are still queued (not yet acknowledged by an agent session).
 */
export async function getQueuedMessages(
	api: ApiClient,
	taskId: string,
): Promise<UiMessage[]> {
	const { messages } = await api.getTaskMessages(taskId);
	return messages.filter((m) => m.isQueued === true);
}
