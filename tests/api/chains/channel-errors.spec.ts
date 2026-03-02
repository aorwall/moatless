/**
 * Standalone: Channel Error Cases (channels.md)
 *
 * Independent error scenario tests for channel operations.
 *
 * CHAN-50: Webhook to non-existent config
 * CHAN-51: Webhook to inactive config
 * CHAN-52: Webhook with invalid signature
 * CHAN-53: Create config with non-existent repository
 * CHAN-54: Webhook with missing text field
 */

import { test, expect } from "../fixtures/api-fixtures";
import {
	createWebhookConfig,
	sendWebhookMessage,
} from "../steps";

// ─── CHAN-50: Webhook to non-existent config ──────────────────────────

test("CHAN-50: Webhook to non-existent config returns error", async ({ api }) => {
	const response = await api.sendWebhookMessage("non-existent-config-id", {
		text: "This should fail",
	});

	expect(response.status).toBe(404);
});

// ─── CHAN-51: Webhook to inactive config ──────────────────────────────

test("CHAN-51: Webhook to inactive config is ignored", async ({ api }) => {
	const repo = await api.ensureTemplateRepository();

	const { configId } = await createWebhookConfig(api, repo.id, {
		sourceId: `inactive-${Date.now()}`,
	});

	// Deactivate the config
	await api.updateChannelConfig(configId, { active: false });

	// Send message — should be accepted but ignored
	const response = await api.sendWebhookMessage(configId, {
		text: "This should be ignored",
	});

	expect(response.status).toBe(200);

	// Cleanup
	await api.deleteChannelConfig(configId).catch(() => {});
});

// ─── CHAN-52: Webhook with invalid signature ──────────────────────────

test("CHAN-52: Webhook with invalid signature is rejected", async ({ api }) => {
	const repo = await api.ensureTemplateRepository();

	const { configId } = await createWebhookConfig(api, repo.id, {
		sourceId: `sig-test-${Date.now()}`,
		providerConfig: {
			webhook_secret: "my-secret-key",
		},
	});

	// Send message with invalid signature
	const response = await api.sendWebhookMessageRaw(
		configId,
		{ text: "This should fail" },
		{ "X-Webhook-Signature": "sha256=invalid" },
	);

	expect(response.status).toBe(401);

	// Cleanup
	await api.deleteChannelConfig(configId).catch(() => {});
});

// ─── CHAN-53: Create config with non-existent repository ──────────────

test("CHAN-53: Create config with non-existent repository returns error", async ({ api }) => {
	try {
		await api.createChannelConfig({
			provider: "webhook",
			sourceId: `no-repo-${Date.now()}`,
			repositoryId: "non-existent-repo-id",
			routingMode: "per_message",
		});
		// Should have thrown
		expect(true).toBe(false);
	} catch (error: unknown) {
		const message = (error as Error).message;
		expect(message).toContain("400");
	}
});

// ─── CHAN-54: Webhook with missing text field ─────────────────────────

test("CHAN-54: Webhook with missing text field returns error", async ({ api }) => {
	const repo = await api.ensureTemplateRepository();

	const { configId } = await createWebhookConfig(api, repo.id, {
		sourceId: `no-text-${Date.now()}`,
	});

	// Send message without text field
	const response = await api.sendWebhookMessageRaw(
		configId,
		{ threadId: "some-thread" },
	);

	expect(response.status).toBe(400);

	// Cleanup
	await api.deleteChannelConfig(configId).catch(() => {});
});
