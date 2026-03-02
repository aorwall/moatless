/**
 * Chain 5: Cross-Channel Response Routing (channels.md)
 *
 * Tests that a trigger config can route outbound replies to a different
 * response config's provider.
 *
 * Chain:
 *   CHAN-40 → CHAN-41 → CHAN-42
 */

import { test, expect } from "../fixtures/api-fixtures";
import { ChainContext } from "../context/chain-context";
import {
	createWebhookConfig,
	linkResponseConfig,
	sendWebhookMessage,
} from "../steps";

// Helper: wait briefly for async webhook processing
async function waitForProcessing(ms = 1000): Promise<void> {
	await new Promise((resolve) => setTimeout(resolve, ms));
}

test.describe("Chain 5: Cross-Channel Response Routing", () => {
	test.describe.configure({ mode: "serial" });

	const ctx = new ChainContext();

	test.beforeAll(async ({ api }) => {
		const repo = await api.ensureTemplateRepository();
		ctx.repositoryId = repo.id;
	});

	test.afterAll(async ({ api }) => {
		for (const key of ["triggerConfigId", "responseConfigId"]) {
			const id = ctx.get<string>(key);
			if (id) {
				await api.deleteChannelConfig(id).catch(() => {});
			}
		}
		for (const taskId of ctx.get<string[]>("taskIds") ?? []) {
			await api.stopTask(taskId).catch(() => {});
			await api.deleteTask(taskId).catch(() => {});
		}
	});

	// ─── CHAN-40: Create two webhook configs (trigger + response) ──────

	test("CHAN-40: Create two webhook configs (trigger + response)", async ({ api }) => {
		const trigger = await createWebhookConfig(api, ctx.repositoryId!, {
			sourceId: `trigger-${Date.now()}`,
			sourceName: "Trigger Webhook",
		});

		const response = await createWebhookConfig(api, ctx.repositoryId!, {
			sourceId: `response-${Date.now()}`,
			sourceName: "Response Webhook",
			providerConfig: {
				callback_url: "https://httpbin.org/post",
			},
		});

		expect(trigger.config.active).toBe(true);
		expect(response.config.active).toBe(true);

		ctx.set("triggerConfigId", trigger.configId);
		ctx.set("responseConfigId", response.configId);
		ctx.set("taskIds", []);
	});

	// ─── CHAN-41: Link trigger config to response config ───────────────

	test("CHAN-41: Link trigger config to response config", async ({ api }) => {
		const updated = await linkResponseConfig(
			api,
			ctx.get<string>("triggerConfigId")!,
			ctx.get<string>("responseConfigId")!,
		);

		expect(updated.responseConfigId).toBe(ctx.get<string>("responseConfigId"));
	});

	// ─── CHAN-42: Inbound on trigger creates task ──────────────────────

	test("CHAN-42: Inbound on trigger creates task with cross-channel routing", async ({ api }) => {
		const response = await sendWebhookMessage(
			api,
			ctx.get<string>("triggerConfigId")!,
			"Cross-channel test message",
		);

		expect(response.status).toBe(200);

		await waitForProcessing();

		// Verify task was created
		const tasks = await api.listTasks({ repositoryId: ctx.repositoryId });
		expect(tasks.items.length).toBeGreaterThan(0);

		const task = tasks.items[0];
		ctx.set("taskIds", [task.taskId]);

		// Verify the trigger config still has response_config_id
		const triggerConfig = await api.getChannelConfig(ctx.get<string>("triggerConfigId")!);
		expect(triggerConfig.responseConfigId).toBe(ctx.get<string>("responseConfigId"));
	});
});
