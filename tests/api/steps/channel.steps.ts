/**
 * Channel step functions (CHAN-01 through CHAN-54).
 *
 * Each function performs an action and returns a result.
 * Tests add assertions; preconditions skip them.
 */

import type {
	ApiClient,
	TaskTriggerFixtureResponse,
} from "../../shared/api-client";

// =============================================================================
// CHAN-01: Create webhook task trigger fixture
// =============================================================================

export async function createWebhookTriggerFixture(
	api: ApiClient,
	repositoryId: string,
	options?: {
		sourceId?: string;
		sourceName?: string;
		routingMode?: string;
		promptTemplate?: string;
		providerConfig?: Record<string, unknown>;
	},
): Promise<{ triggerId: string; config: TaskTriggerFixtureResponse }> {
	const config = await api.createTaskTriggerFixture({
		provider: "webhook",
		sourceId: options?.sourceId ?? `webhook-${Date.now()}`,
		sourceName: options?.sourceName,
		repositoryId,
		routingMode: options?.routingMode ?? "per_message",
		promptTemplate: options?.promptTemplate,
		providerConfig: options?.providerConfig,
	});
	return { triggerId: config.id, config };
}

// =============================================================================
// CHAN-10/20/30: Create fixture with specific routing mode
// =============================================================================

export async function createWebhookTriggerFixtureWithRouting(
	api: ApiClient,
	repositoryId: string,
	routingMode: string,
	options?: {
		sourceId?: string;
		providerConfig?: Record<string, unknown>;
	},
): Promise<{ triggerId: string; config: TaskTriggerFixtureResponse }> {
	return createWebhookTriggerFixture(api, repositoryId, {
		routingMode,
		sourceId: options?.sourceId,
		providerConfig: options?.providerConfig,
	});
}

// =============================================================================
// CHAN-11: Send message via webhook
// =============================================================================

export async function sendWebhookMessage(
	api: ApiClient,
	triggerId: string,
	text: string,
	options?: {
		threadId?: string;
		userId?: string;
		metadata?: Record<string, unknown>;
	},
): Promise<{ response: Response; taskId: string | null }> {
	const response = await api.sendWebhookMessage(triggerId, {
		text,
		threadId: options?.threadId,
		userId: options?.userId,
		metadata: options?.metadata,
	});

	let taskId: string | null = null;
	if (response.ok) {
		const body = await response.json();
		taskId = body.taskId ?? null;
	}

	return { response, taskId };
}
