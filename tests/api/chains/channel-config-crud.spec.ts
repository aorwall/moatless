/**
 * Chain 1: Channel Config CRUD (channels.md)
 *
 * Pure API chain — tests channel config creation, retrieval, updates, and deletion.
 *
 * Chain:
 *   CHAN-01 → CHAN-01.1 → CHAN-02 → CHAN-03 → CHAN-04 → CHAN-05 → CHAN-05.1
 */

import { test, expect } from "../fixtures/api-fixtures";
import { ChainContext } from "../context/chain-context";
import { createWebhookConfig } from "../steps";

test.describe("Chain 1: Channel Config CRUD", () => {
	test.describe.configure({ mode: "serial" });

	const ctx = new ChainContext();

	test.beforeAll(async ({ api }) => {
		const repo = await api.ensureTemplateRepository();
		ctx.repositoryId = repo.id;
		ctx.repository = repo;
	});

	test.afterAll(async ({ api }) => {
		const configId = ctx.get<string>("configId");
		if (configId) {
			await api.deleteChannelConfig(configId).catch(() => {});
		}
	});

	// ─── CHAN-01: Create webhook channel config ────────────────────────

	test("CHAN-01: Create webhook channel config", async ({ api }) => {
		const result = await createWebhookConfig(api, ctx.repositoryId!, {
			sourceId: `crud-test-${Date.now()}`,
			sourceName: "CRUD Test Webhook",
		});

		expect(result.config.active).toBe(true);
		expect(result.configId).toBeTruthy();
		expect(result.config.provider).toBe("webhook");
		expect(result.config.repositoryId).toBe(ctx.repositoryId);

		ctx.set("configId", result.configId);
		ctx.set("config", result.config);
	});

	// ─── CHAN-01.1: Config appears in listing ──────────────────────────

	test("CHAN-01.1: Config appears in listing", async ({ api }) => {
		const configs = await api.listChannelConfigs();

		const found = configs.some((c) => c.id === ctx.get<string>("configId"));
		expect(found).toBe(true);
	});

	// ─── CHAN-02: Get channel config by ID ─────────────────────────────

	test("CHAN-02: Get channel config by ID", async ({ api }) => {
		const config = await api.getChannelConfig(ctx.get<string>("configId")!);

		expect(config.id).toBe(ctx.get<string>("configId"));
		expect(config.provider).toBe("webhook");
		expect(config.routingMode).toBe("per_message");
		expect(config.active).toBe(true);
	});

	// ─── CHAN-03: Update channel config ────────────────────────────────

	test("CHAN-03: Update channel config", async ({ api }) => {
		const updated = await api.updateChannelConfig(ctx.get<string>("configId")!, {
			sourceName: "Updated CRUD Webhook",
			promptTemplate: "Please help: {message}",
		});

		expect(updated.sourceName).toBe("Updated CRUD Webhook");
		expect(updated.promptTemplate).toBe("Please help: {message}");
		expect(updated.routingMode).toBe("per_message");
	});

	// ─── CHAN-04: Deactivate channel config ────────────────────────────

	test("CHAN-04: Deactivate channel config", async ({ api }) => {
		const updated = await api.updateChannelConfig(ctx.get<string>("configId")!, {
			active: false,
		});

		expect(updated.active).toBe(false);

		// Still appears in listing
		const configs = await api.listChannelConfigs();
		const found = configs.some((c) => c.id === ctx.get<string>("configId"));
		expect(found).toBe(true);
	});

	// ─── CHAN-05: Delete channel config ────────────────────────────────

	test("CHAN-05: Delete channel config", async ({ api }) => {
		const response = await api.deleteChannelConfigRaw(ctx.get<string>("configId")!);

		expect(response.status).toBe(204);
	});

	// ─── CHAN-05.1: Config removed from listing ───────────────────────

	test("CHAN-05.1: Config removed from listing", async ({ api }) => {
		const configs = await api.listChannelConfigs();

		const found = configs.some((c) => c.id === ctx.get<string>("configId"));
		expect(found).toBe(false);

		ctx.set("configId", undefined);
	});
});
