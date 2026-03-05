/**
 * Chain 6: Webhook Outbound Callback (channels.md)
 *
 * Tests that the backend delivers outbound callback POSTs to the
 * webhook config's callback_url when an agent completes a task.
 *
 * Requires a running sandbox environment.
 * Gated behind CHANNEL_CALLBACK_E2E=1 env var.
 *
 * Chain:
 *   CHAN-60 → CHAN-61 → CHAN-62 → CHAN-63
 */

import { test, expect } from "../fixtures/api-fixtures";
import { ChainContext } from "../context/chain-context";
import { WebhookCallbackServer } from "../../shared/webhook-callback-server";
import {
	createWebhookConfig,
	sendWebhookMessage,
} from "../steps";
import { waitForAgentComplete } from "../../shared/wait-helpers";

// Gate: requires sandbox running
test.skip(
	!process.env.CHANNEL_CALLBACK_E2E,
	"Set CHANNEL_CALLBACK_E2E=1 to run callback tests (requires sandbox)",
);

// Helper: wait briefly for async webhook processing
async function waitForProcessing(ms = 2000): Promise<void> {
	await new Promise((resolve) => setTimeout(resolve, ms));
}

test.describe("Chain 6: Webhook Outbound Callback", () => {
	test.describe.configure({ mode: "serial" });

	const ctx = new ChainContext();
	let callbackServer: WebhookCallbackServer;

	test.beforeAll(async ({ api }) => {
		// Start the callback server
		callbackServer = new WebhookCallbackServer({
			debug: !!process.env.DEBUG_CALLBACK,
		});
		await callbackServer.start();

		// Set up repository
		const repo = await api.ensureTemplateRepository();
		ctx.repositoryId = repo.id;
	});

	test.afterAll(async ({ api }) => {
		// Clean up callback server
		await callbackServer?.stop();

		// Clean up channel config
		const configId = ctx.get<string>("configId");
		if (configId) {
			await api.deleteChannelConfig(configId).catch(() => {});
		}

		// Clean up tasks
		for (const taskId of ctx.get<string[]>("taskIds") ?? []) {
			await api.stopTask(taskId).catch(() => {});
			await api.deleteTask(taskId).catch(() => {});
		}
	});

	// ─── CHAN-60: Create webhook config with callback URL ────────────────

	test("CHAN-60: Create webhook config with callback URL", async ({ api }) => {
		const result = await createWebhookConfig(api, ctx.repositoryId!, {
			sourceId: `callback-test-${Date.now()}`,
			providerConfig: {
				callback_url: callbackServer.url,
			},
		});

		expect(result.config.active).toBe(true);
		expect(result.configId).toBeTruthy();

		ctx.set("configId", result.configId);
		ctx.set("taskIds", []);
	});

	// ─── CHAN-61: Send inbound message — agent starts ────────────────────

	test("CHAN-61: Send inbound message — agent starts", async ({ api }) => {
		const { response, taskId } = await sendWebhookMessage(
			api,
			ctx.get<string>("configId")!,
			"Say hello world. Keep it brief.",
		);

		expect(response.status).toBe(200);
		expect(taskId).toBeTruthy();

		ctx.set("taskId", taskId);
		ctx.set("taskIds", [taskId!]);
	});

	// ─── CHAN-62: Wait for agent completion ──────────────────────────────

	test("CHAN-62: Wait for agent completion", async ({ api }) => {
		const completed = await waitForAgentComplete(
			api,
			ctx.get<string>("taskId")!,
			{
				timeout: 120_000,
				onProgress: (msg) => console.log(`  [CHAN-62] ${msg}`),
			},
		);

		expect(completed).toBe(true);
	});

	// ─── CHAN-63: Callback server received outbound POST ─────────────────

	test("CHAN-63: Callback server received outbound POST", async () => {
		// The callback may have already arrived during agent completion,
		// or we may need to wait a moment for the event consumer to process it
		const callback = await callbackServer.waitForCallback(15_000);

		expect(callback.method).toBe("POST");
		expect(callback.headers["content-type"]).toContain("application/json");
		expect(callback.body).toMatchObject({
			task_id: expect.any(String),
			text: expect.any(String),
			is_error: false,
		});

		// Verify the task_id matches our task
		const body = callback.body as Record<string, unknown>;
		expect(body.task_id).toBe(ctx.get<string>("taskId"));
	});
});
