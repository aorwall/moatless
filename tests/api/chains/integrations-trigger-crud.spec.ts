/**
 * Chain 1: Task Route and Trigger CRUD (integrations-triggers-bindings.md)
 *
 * Pure API chain — tests task trigger fixture creation, retrieval, updates, and deletion.
 *
 * Chain:
 *   INT-01 → INT-01.1 → INT-02 → INT-03 → INT-04 → INT-05 → INT-05.1
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

	// ─── INT-01: Create webhook task trigger fixture ────────────────────────

	test("INT-01: Create webhook task trigger fixture", async ({ api }) => {
		const result = await createWebhookTriggerFixture(api, ctx.repositoryId!, {
			sourceId: `crud-test-${Date.now()}`,
			sourceName: "CRUD Test Webhook",
		});

		expect(result.config.active).toBe(true);
		expect(result.triggerId).toBeTruthy();
		expect(result.config.adapterKind).toBe("webhook");
		expect(result.config.repositoryId).toBe(ctx.repositoryId);

		ctx.set("triggerId", result.triggerId);
		ctx.set("config", result.config);
	});

	// ─── INT-01.1: Config appears in listing ──────────────────────────

	test("INT-01.1: Config appears in listing", async ({ api }) => {
		const configs = await api.listTaskTriggerFixtures();

		const found = configs.some((c) => c.id === ctx.get<string>("triggerId"));
		expect(found).toBe(true);
	});

	// ─── INT-02: Get task trigger fixture by ID ─────────────────────────────

	test("INT-02: Get task trigger fixture by ID", async ({ api }) => {
		const config = await api.getTaskTriggerFixture(ctx.get<string>("triggerId")!);

		expect(config.id).toBe(ctx.get<string>("triggerId"));
		expect(config.adapterKind).toBe("webhook");
		expect(config.routingMode).toBe("by_subject");
		expect(config.active).toBe(true);
	});

	// ─── INT-03: Update task trigger fixture ────────────────────────────────

	test("INT-03: Update task trigger fixture", async ({ api }) => {
		const updated = await api.updateTaskTriggerFixture(ctx.get<string>("triggerId")!, {
			sourceName: "Updated CRUD Webhook",
			promptTemplate: "Please help: {message}",
		});

		expect(updated.sourceName).toBe("Updated CRUD Webhook");
		expect(updated.promptTemplate).toBe("Please help: {message}");
		expect(updated.routingMode).toBe("by_subject");
	});

	// ─── INT-04: Deactivate task trigger fixture ────────────────────────────

	test("INT-04: Deactivate task trigger fixture", async ({ api }) => {
		const updated = await api.updateTaskTriggerFixture(ctx.get<string>("triggerId")!, {
			active: false,
		});

		expect(updated.active).toBe(false);

		// Still appears in listing
		const configs = await api.listTaskTriggerFixtures();
		const found = configs.some((c) => c.id === ctx.get<string>("triggerId"));
		expect(found).toBe(true);
	});

	// ─── INT-05: Delete task trigger fixture ────────────────────────────────

	test("INT-05: Delete task trigger fixture", async ({ api }) => {
		const response = await api.deleteTaskTriggerFixtureRaw(ctx.get<string>("triggerId")!);

		expect(response.status).toBe(204);
	});

	// ─── INT-05.1: Config removed from listing ───────────────────────

	test("INT-05.1: Config removed from listing", async ({ api }) => {
		const configs = await api.listTaskTriggerFixtures();

		const found = configs.some((c) => c.id === ctx.get<string>("triggerId"));
		expect(found).toBe(false);

		ctx.set("triggerId", undefined);
	});
});
