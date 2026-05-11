/**
 * Standalone: Integration Error Cases (integrations-triggers-bindings.md)
 *
 * Independent error scenario tests for integration operations.
 *
 * INT-50: Webhook to non-existent config
 * INT-51: Webhook to inactive config
 * INT-52: Webhook with invalid signature
 * INT-53: Create config with non-existent repository
 * INT-54: Webhook with missing text field
 */

import { test, expect } from "../fixtures/api-fixtures";
import {
	createWebhookTriggerFixture,
	sendWebhookMessage,
} from "../steps";

// ─── INT-50: Webhook to non-existent config ──────────────────────────

test("INT-50: Webhook to non-existent config returns error", async ({ api }) => {
	const response = await api.sendWebhookMessage("non-existent-config-id", {
		text: "This should fail",
	});

	expect(response.status).toBe(404);
});

// ─── INT-51: Webhook to inactive config ──────────────────────────────

test("INT-51: Webhook to inactive config is ignored", async ({ api }) => {
	const repo = await api.ensureTemplateRepository();

	const { triggerId } = await createWebhookTriggerFixture(api, repo.id, {
		sourceId: `inactive-${Date.now()}`,
	});

	// Deactivate the config
	await api.updateTaskTriggerFixture(triggerId, { active: false });

	// Send message — should be accepted but ignored
	const response = await api.sendWebhookMessage(triggerId, {
		text: "This should be ignored",
	});

	expect(response.status).toBe(200);

	// Cleanup
	await api.deleteTaskTriggerFixture(triggerId).catch(() => {});
});

// ─── INT-52: Webhook with invalid signature ──────────────────────────

test("INT-52: Webhook with invalid signature is rejected", async ({ api }) => {
	const repo = await api.ensureTemplateRepository();

	const { triggerId } = await createWebhookTriggerFixture(api, repo.id, {
		sourceId: `sig-test-${Date.now()}`,
		providerConfig: {
			webhook_secret: "my-secret-key",
		},
	});

	// Send message with invalid signature
	const response = await api.sendWebhookMessageRaw(
		triggerId,
		{ text: "This should fail" },
		{ "X-Webhook-Signature": "sha256=invalid" },
	);

	expect(response.status).toBe(401);

	// Cleanup
	await api.deleteTaskTriggerFixture(triggerId).catch(() => {});
});

// ─── INT-53: Create config with non-existent repository ──────────────

test("INT-53: Create config with non-existent repository returns error", async ({ api }) => {
	try {
		await api.createTaskTriggerFixture({
			adapterKind: "webhook",
			sourceId: `no-repo-${Date.now()}`,
			repositoryId: "non-existent-repo-id",
			routingMode: "by_subject",
		});
		// Should have thrown
		expect(true).toBe(false);
	} catch (error: unknown) {
		const message = (error as Error).message;
		expect(message).toContain("400");
	}
});

// ─── INT-54: Webhook with missing text field ─────────────────────────

test("INT-54: Webhook with missing text field returns error", async ({ api }) => {
	const repo = await api.ensureTemplateRepository();

	const { triggerId } = await createWebhookTriggerFixture(api, repo.id, {
		sourceId: `no-text-${Date.now()}`,
	});

	// Send message without text field
	const response = await api.sendWebhookMessageRaw(
		triggerId,
		{ threadId: "some-thread" },
	);

	expect(response.status).toBe(400);

	// Cleanup
	await api.deleteTaskTriggerFixture(triggerId).catch(() => {});
});
