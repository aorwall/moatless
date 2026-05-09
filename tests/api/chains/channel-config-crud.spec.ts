/**
 * Chain 1: Task Route and Trigger CRUD (channels.md)
 *
 * Pure API chain — tests task trigger fixture creation, retrieval, updates, and deletion.
 *
 * Chain:
 *   CHAN-01 → CHAN-01.1 → CHAN-02 → CHAN-03 → CHAN-04 → CHAN-05 → CHAN-05.1
 */

import { test, expect } from "../fixtures/api-fixtures";
import { ChainContext } from "../context/chain-context";
import { createWebhookTriggerFixture } from "../steps";

test.describe("Chain 1: Task Route and Trigger CRUD", () => {
	test.describe.configure({ mode: "serial" });

	const ctx = new ChainContext();

	test.beforeAll(async ({ api }) => {
		const repo = await api.ensureTemplateRepository();
		ctx.repositoryId = repo.id;
		ctx.repository = repo;
	});

	test.afterAll(async ({ api }) => {
		const triggerId = ctx.get<string>("triggerId");
		if (triggerId) {
			await api.deleteTaskTriggerFixture(triggerId).catch(() => {});
		}
	});

	// ─── CHAN-01: Create webhook task trigger fixture ────────────────────────

	test("CHAN-01: Create webhook task trigger fixture", async ({ api }) => {
		const result = await createWebhookTriggerFixture(api, ctx.repositoryId!, {
			sourceId: `crud-test-${Date.now()}`,
			sourceName: "CRUD Test Webhook",
		});

		expect(result.config.active).toBe(true);
		expect(result.triggerId).toBeTruthy();
		expect(result.config.provider).toBe("webhook");
		expect(result.config.repositoryId).toBe(ctx.repositoryId);

		ctx.set("triggerId", result.triggerId);
		ctx.set("config", result.config);
	});

	// ─── CHAN-01.1: Config appears in listing ──────────────────────────

	test("CHAN-01.1: Config appears in listing", async ({ api }) => {
		const configs = await api.listTaskTriggerFixtures();

		const found = configs.some((c) => c.id === ctx.get<string>("triggerId"));
		expect(found).toBe(true);
	});

	// ─── CHAN-02: Get task trigger fixture by ID ─────────────────────────────

	test("CHAN-02: Get task trigger fixture by ID", async ({ api }) => {
		const config = await api.getTaskTriggerFixture(ctx.get<string>("triggerId")!);

		expect(config.id).toBe(ctx.get<string>("triggerId"));
		expect(config.provider).toBe("webhook");
		expect(config.routingMode).toBe("per_message");
		expect(config.active).toBe(true);
	});

	// ─── CHAN-03: Update task trigger fixture ────────────────────────────────

	test("CHAN-03: Update task trigger fixture", async ({ api }) => {
		const updated = await api.updateTaskTriggerFixture(ctx.get<string>("triggerId")!, {
			sourceName: "Updated CRUD Webhook",
			promptTemplate: "Please help: {message}",
		});

		expect(updated.sourceName).toBe("Updated CRUD Webhook");
		expect(updated.promptTemplate).toBe("Please help: {message}");
		expect(updated.routingMode).toBe("per_message");
	});

	// ─── CHAN-04: Deactivate task trigger fixture ────────────────────────────

	test("CHAN-04: Deactivate task trigger fixture", async ({ api }) => {
		const updated = await api.updateTaskTriggerFixture(ctx.get<string>("triggerId")!, {
			active: false,
		});

		expect(updated.active).toBe(false);

		// Still appears in listing
		const configs = await api.listTaskTriggerFixtures();
		const found = configs.some((c) => c.id === ctx.get<string>("triggerId"));
		expect(found).toBe(true);
	});

	// ─── CHAN-05: Delete task trigger fixture ────────────────────────────────

	test("CHAN-05: Delete task trigger fixture", async ({ api }) => {
		const response = await api.deleteTaskTriggerFixtureRaw(ctx.get<string>("triggerId")!);

		expect(response.status).toBe(204);
	});

	// ─── CHAN-05.1: Config removed from listing ───────────────────────

	test("CHAN-05.1: Config removed from listing", async ({ api }) => {
		const configs = await api.listTaskTriggerFixtures();

		const found = configs.some((c) => c.id === ctx.get<string>("triggerId"));
		expect(found).toBe(false);

		ctx.set("triggerId", undefined);
	});
});
