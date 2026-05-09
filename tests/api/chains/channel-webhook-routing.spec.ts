/**
 * Chains 2-4: Webhook Inbound Routing (channels.md)
 *
 * Tests webhook inbound routing with all three routing modes:
 * per_message, ongoing, and thread_based.
 *
 * Chain 2: CHAN-10 → CHAN-11 → CHAN-11.1 → CHAN-11.2 → CHAN-12 → CHAN-13
 * Chain 3: CHAN-20 → CHAN-21 → CHAN-22 → CHAN-23
 * Chain 4: CHAN-30 → CHAN-31 → CHAN-32 → CHAN-33
 */

import { test, expect } from "../fixtures/api-fixtures";
import { ChainContext } from "../context/chain-context";
import {
	createWebhookTriggerFixtureWithRouting,
	sendWebhookMessage,
} from "../steps";

// Helper: wait briefly for async webhook processing
async function waitForProcessing(ms = 1000): Promise<void> {
	await new Promise((resolve) => setTimeout(resolve, ms));
}

// =============================================================================
// Chain 2: Webhook Inbound (per_message routing)
// =============================================================================

test.describe("Chain 2: Webhook per_message routing", () => {
	test.describe.configure({ mode: "serial" });

	const ctx = new ChainContext();

	test.beforeAll(async ({ api }) => {
		const repo = await api.ensureTemplateRepository();
		ctx.repositoryId = repo.id;
	});

	test.afterAll(async ({ api }) => {
		const triggerId = ctx.get<string>("triggerId");
		if (triggerId) {
			await api.deleteTaskTriggerFixture(triggerId).catch(() => {});
		}
		// Clean up tasks
		for (const taskId of ctx.get<string[]>("taskIds") ?? []) {
			await api.stopTask(taskId).catch(() => {});
			await api.deleteTask(taskId).catch(() => {});
		}
	});

	// ─── CHAN-10: Create webhook config with per_message routing ───────

	test("CHAN-10: Create webhook config with per_message routing", async ({ api }) => {
		const result = await createWebhookTriggerFixtureWithRouting(
			api,
			ctx.repositoryId!,
			"per_message",
			{ sourceId: `per-msg-${Date.now()}` },
		);

		expect(result.config.active).toBe(true);
		expect(result.config.routingMode).toBe("per_message");

		ctx.set("triggerId", result.triggerId);
		ctx.set("taskIds", []);
	});

	// ─── CHAN-11: Send message via webhook — task created ──────────────

	test("CHAN-11: Send message via webhook — task created", async ({ api }) => {
		const { response, taskId } = await sendWebhookMessage(
			api,
			ctx.get<string>("triggerId")!,
			"First per_message webhook test",
		);

		expect(response.status).toBe(200);
		expect(taskId).toBeTruthy();

		ctx.set("firstTaskId", taskId);
		ctx.set("taskIds", [taskId!]);
	});

	// ─── CHAN-11.1: Task is in open state ──────────────────────────────

	test("CHAN-11.1: Task is in open state", async ({ api }) => {
		const task = await api.getTask(ctx.get<string>("firstTaskId")!);

		expect(task.status).toBe("open");
	});

	// ─── CHAN-11.2: Task linked to correct repository ──────────────────

	test("CHAN-11.2: Task linked to correct repository", async ({ api }) => {
		const task = await api.getTask(ctx.get<string>("firstTaskId")!);

		expect(task.repositoryId).toBe(ctx.repositoryId);
	});

	// ─── CHAN-12: Send second message — new task created ───────────────

	test("CHAN-12: Send second message — new task created", async ({ api }) => {
		const { response, taskId } = await sendWebhookMessage(
			api,
			ctx.get<string>("triggerId")!,
			"Second per_message webhook test",
		);

		expect(response.status).toBe(200);
		expect(taskId).toBeTruthy();
		expect(taskId).not.toBe(ctx.get<string>("firstTaskId"));

		ctx.set("taskIds", [
			...(ctx.get<string[]>("taskIds") ?? []),
			taskId!,
		]);
	});

	// ─── CHAN-13: Send message with thread_id — new task ──────────────

	test("CHAN-13: Send message with thread_id — no existing thread, new task", async ({ api }) => {
		const { response, taskId } = await sendWebhookMessage(
			api,
			ctx.get<string>("triggerId")!,
			"Message with new thread",
			{ threadId: `new-thread-${Date.now()}` },
		);

		expect(response.status).toBe(200);
		expect(taskId).toBeTruthy();
		expect(taskId).not.toBe(ctx.get<string>("firstTaskId"));

		ctx.set("taskIds", [
			...(ctx.get<string[]>("taskIds") ?? []),
			taskId!,
		]);
	});
});

// =============================================================================
// Chain 3: Webhook Inbound (ongoing routing)
// =============================================================================

test.describe("Chain 3: Webhook ongoing routing", () => {
	test.describe.configure({ mode: "serial" });

	const ctx = new ChainContext();

	test.beforeAll(async ({ api }) => {
		const repo = await api.ensureTemplateRepository();
		ctx.repositoryId = repo.id;
	});

	test.afterAll(async ({ api }) => {
		const triggerId = ctx.get<string>("triggerId");
		if (triggerId) {
			await api.deleteTaskTriggerFixture(triggerId).catch(() => {});
		}
		for (const taskId of ctx.get<string[]>("taskIds") ?? []) {
			await api.stopTask(taskId).catch(() => {});
			await api.deleteTask(taskId).catch(() => {});
		}
	});

	// ─── CHAN-20: Create webhook config with ongoing routing ───────────

	test("CHAN-20: Create webhook config with ongoing routing", async ({ api }) => {
		const result = await createWebhookTriggerFixtureWithRouting(
			api,
			ctx.repositoryId!,
			"ongoing",
			{ sourceId: `ongoing-${Date.now()}` },
		);

		expect(result.config.active).toBe(true);
		expect(result.config.routingMode).toBe("ongoing");

		ctx.set("triggerId", result.triggerId);
		ctx.set("taskIds", []);
	});

	// ─── CHAN-21: First message creates ongoing task ───────────────────

	test("CHAN-21: First message creates ongoing task", async ({ api }) => {
		const { response, taskId } = await sendWebhookMessage(
			api,
			ctx.get<string>("triggerId")!,
			"First ongoing message",
		);

		expect(response.status).toBe(200);
		expect(taskId).toBeTruthy();

		await waitForProcessing();

		// Check that config now has ongoing_task_id
		const config = await api.getTaskTriggerFixture(ctx.get<string>("triggerId")!);
		expect(config.ongoingTaskId).toBe(taskId);

		ctx.set("ongoingTaskId", taskId);
		ctx.set("taskIds", [taskId!]);
	});

	// ─── CHAN-22: Second message routes to same task ───────────────────

	test("CHAN-22: Second message routes to same task", async ({ api }) => {
		const { response, taskId } = await sendWebhookMessage(
			api,
			ctx.get<string>("triggerId")!,
			"Second ongoing message",
		);

		expect(response.status).toBe(200);

		// Webhook should return the same ongoing task ID
		expect(taskId).toBe(ctx.get<string>("ongoingTaskId"));

		// Config should still point to same task
		const config = await api.getTaskTriggerFixture(ctx.get<string>("triggerId")!);
		expect(config.ongoingTaskId).toBe(ctx.get<string>("ongoingTaskId"));
	});

	// ─── CHAN-23: After task closed, next message creates new task ─────

	test("CHAN-23: After task closed, next message creates new task", async ({ api }) => {
		// Close the ongoing task
		await api.closeTask(ctx.get<string>("ongoingTaskId")!);

		const { response, taskId } = await sendWebhookMessage(
			api,
			ctx.get<string>("triggerId")!,
			"Message after close",
		);

		expect(response.status).toBe(200);
		expect(taskId).toBeTruthy();
		expect(taskId).not.toBe(ctx.get<string>("ongoingTaskId"));

		await waitForProcessing();

		// Config should now point to the new task
		const config = await api.getTaskTriggerFixture(ctx.get<string>("triggerId")!);
		expect(config.ongoingTaskId).toBe(taskId);

		ctx.set("taskIds", [
			...(ctx.get<string[]>("taskIds") ?? []),
			taskId!,
		]);
	});
});

// =============================================================================
// Chain 4: Webhook Inbound (thread_based routing)
// =============================================================================

test.describe("Chain 4: Webhook thread_based routing", () => {
	test.describe.configure({ mode: "serial" });

	const ctx = new ChainContext();

	test.beforeAll(async ({ api }) => {
		const repo = await api.ensureTemplateRepository();
		ctx.repositoryId = repo.id;
	});

	test.afterAll(async ({ api }) => {
		const triggerId = ctx.get<string>("triggerId");
		if (triggerId) {
			await api.deleteTaskTriggerFixture(triggerId).catch(() => {});
		}
		for (const taskId of ctx.get<string[]>("taskIds") ?? []) {
			await api.stopTask(taskId).catch(() => {});
			await api.deleteTask(taskId).catch(() => {});
		}
	});

	// ─── CHAN-30: Create webhook config with thread_based routing ──────

	test("CHAN-30: Create webhook config with thread_based routing", async ({ api }) => {
		const result = await createWebhookTriggerFixtureWithRouting(
			api,
			ctx.repositoryId!,
			"thread_based",
			{ sourceId: `thread-${Date.now()}` },
		);

		expect(result.config.active).toBe(true);
		expect(result.config.routingMode).toBe("thread_based");

		ctx.set("triggerId", result.triggerId);
		ctx.set("taskIds", []);
	});

	// ─── CHAN-31: New message creates task ─────────────────────────────

	test("CHAN-31: New message creates task", async ({ api }) => {
		const threadId = `thread-a-${Date.now()}`;

		const { response, taskId } = await sendWebhookMessage(
			api,
			ctx.get<string>("triggerId")!,
			"First thread message",
			{ threadId },
		);

		expect(response.status).toBe(200);
		expect(taskId).toBeTruthy();

		ctx.set("threadId", threadId);
		ctx.set("firstTaskId", taskId);
		ctx.set("taskIds", [taskId!]);
	});

	// ─── CHAN-32: Reply in same thread routes to same task ─────────────

	test("CHAN-32: Reply in same thread routes to same task", async ({ api }) => {
		const { response, taskId } = await sendWebhookMessage(
			api,
			ctx.get<string>("triggerId")!,
			"Reply in same thread",
			{ threadId: ctx.get<string>("threadId") },
		);

		expect(response.status).toBe(200);

		// Should route to the same task, not create a new one
		expect(taskId).toBe(ctx.get<string>("firstTaskId"));
	});

	// ─── CHAN-33: New thread creates new task ──────────────────────────

	test("CHAN-33: New thread creates new task", async ({ api }) => {
		const newThreadId = `thread-b-${Date.now()}`;

		const { response, taskId } = await sendWebhookMessage(
			api,
			ctx.get<string>("triggerId")!,
			"Message in new thread",
			{ threadId: newThreadId },
		);

		expect(response.status).toBe(200);
		expect(taskId).toBeTruthy();
		expect(taskId).not.toBe(ctx.get<string>("firstTaskId"));

		ctx.set("taskIds", [
			...(ctx.get<string[]>("taskIds") ?? []),
			taskId!,
		]);
	});
});
