/**
 * Channel step functions (CHAN-01 through CHAN-54).
 *
 * Each function performs an action and returns a result.
 * Tests add assertions; preconditions skip them.
 */

import type {
	ApiClient,
	ChannelConfigResponse,
} from "../../shared/api-client";

// =============================================================================
// CHAN-01: Create webhook channel config
// =============================================================================

export async function createWebhookConfig(
	api: ApiClient,
	repositoryId: string,
	options?: {
		sourceId?: string;
		sourceName?: string;
		routingMode?: string;
		promptTemplate?: string;
		providerConfig?: Record<string, unknown>;
		responseConfigId?: string;
	},
): Promise<{ configId: string; config: ChannelConfigResponse }> {
	const config = await api.createChannelConfig({
		provider: "webhook",
		sourceId: options?.sourceId ?? `webhook-${Date.now()}`,
		sourceName: options?.sourceName,
		repositoryId,
		routingMode: options?.routingMode ?? "per_message",
		promptTemplate: options?.promptTemplate,
		providerConfig: options?.providerConfig,
		responseConfigId: options?.responseConfigId,
	});
	return { configId: config.id, config };
}

// =============================================================================
// CHAN-10/20/30: Create config with specific routing mode
// =============================================================================

export async function createWebhookConfigWithRouting(
	api: ApiClient,
	repositoryId: string,
	routingMode: string,
	options?: {
		sourceId?: string;
		providerConfig?: Record<string, unknown>;
	},
): Promise<{ configId: string; config: ChannelConfigResponse }> {
	return createWebhookConfig(api, repositoryId, {
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
	configId: string,
	text: string,
	options?: {
		threadId?: string;
		userId?: string;
		metadata?: Record<string, unknown>;
	},
): Promise<{ response: Response; taskId: string | null }> {
	const response = await api.sendWebhookMessage(configId, {
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

// =============================================================================
// CHAN-40: Link configs for cross-channel routing
// =============================================================================

export async function linkResponseConfig(
	api: ApiClient,
	configId: string,
	responseConfigId: string,
): Promise<ChannelConfigResponse> {
	return api.updateChannelConfig(configId, { responseConfigId });
}
