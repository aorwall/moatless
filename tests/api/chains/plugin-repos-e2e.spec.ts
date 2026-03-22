/**
 * Chain: Plugin Repos E2E
 *
 * Verifies the full plugin repos flow: create plugin repo, link to repo,
 * provision sandbox with plugin, and verify agent has access to the plugin's skill.
 *
 * Uses: https://github.com/amekala/adspirer-mcp-plugin.git
 * Which contains skill: ad-campaign-best-practices
 *
 * Requires Claude authentication (CLAUDE_CODE_OAUTH_TOKEN or ANTHROPIC_API_KEY).
 *
 * Chain:
 *   PLUGIN-01 -> PLUGIN-01.1 -> PLUGIN-02 -> PLUGIN-02.1 ->
 *   TASK-01 -> SANDBOX-01 -> SANDBOX-01.1 -> AGENT-01 -> AGENT-01.1 -> CLEANUP
 */

import { test, expect } from "../fixtures/api-fixtures";
import { getBaseURL } from "../fixtures/api-fixtures";
import { ChainContext } from "../context/chain-context";
import { SSEEventCollector } from "../../shared/sse-collector";
import {
	createTaskWithMessage,
	waitForSandboxReady,
	waitForAgentTurnComplete,
	getTaskMessages,
} from "../steps";

const hasClaudeAuth =
	!!process.env.CLAUDE_CODE_OAUTH_TOKEN || !!process.env.ANTHROPIC_API_KEY;

const PLUGIN_GIT_URL = "https://github.com/amekala/adspirer-mcp-plugin.git";
const PLUGIN_NAME = "adspirer-mcp-plugin";

test.describe("Chain: Plugin Repos E2E", () => {
	test.describe.configure({ mode: "serial" });

	test.skip(!hasClaudeAuth, "Skipping — no Claude authentication");

	const ctx = new ChainContext();
	let chainSse: SSEEventCollector;
	let pluginRepoId: string;

	test.beforeAll(async ({ api }) => {
		const repo = await api.ensureTemplateRepository();
		ctx.repositoryId = repo.id;
		ctx.repository = repo;

		chainSse = new SSEEventCollector({ debug: !!process.env.DEBUG_SSE });
		await chainSse.start(`${getBaseURL()}/api/v1/events/stream`);
	});

	test.afterAll(async ({ api }) => {
		chainSse?.stop();

		// Unlink plugin from repo (ignore errors)
		if (pluginRepoId && ctx.repositoryId) {
			await api.unlinkPluginFromRepo(ctx.repositoryId, pluginRepoId).catch(() => {});
		}
		// Delete plugin repo
		if (pluginRepoId) {
			await api.deletePluginRepo(pluginRepoId).catch(() => {});
		}
		// Stop + delete task
		if (ctx.taskId) {
			await api.stopTask(ctx.taskId).catch(() => {});
			await new Promise((r) => setTimeout(r, 500));
			await api.deleteTask(ctx.taskId).catch(() => {});
		}
	});

	// --- PLUGIN-01: Create plugin repo ----------------------------------------

	test("PLUGIN-01: Create plugin repo", async ({ api }) => {
		const plugin = await api.createPluginRepo({
			name: PLUGIN_NAME,
			gitUrl: PLUGIN_GIT_URL,
		});

		expect(plugin.id).toBeTruthy();
		expect(plugin.name).toBe(PLUGIN_NAME);
		expect(plugin.gitUrl).toBe(PLUGIN_GIT_URL);

		pluginRepoId = plugin.id;
		console.log(`[Plugin] Created plugin repo: ${plugin.id}`);
	});

	// --- PLUGIN-01.1: List shows new plugin -----------------------------------

	test("PLUGIN-01.1: List plugin repos includes new plugin", async ({ api }) => {
		const plugins = await api.listPluginRepos();
		const found = plugins.find((p) => p.id === pluginRepoId);

		expect(found).toBeDefined();
		expect(found!.name).toBe(PLUGIN_NAME);
	});

	// --- PLUGIN-02: Link plugin to repository ---------------------------------

	test("PLUGIN-02: Link plugin to repository", async ({ api }) => {
		await api.linkPluginToRepo(ctx.repositoryId!, pluginRepoId);
		console.log(`[Plugin] Linked ${pluginRepoId} to repo ${ctx.repositoryId}`);
	});

	// --- PLUGIN-02.1: Verify link appears in repo plugins ---------------------

	test("PLUGIN-02.1: Repository plugins list shows linked plugin", async ({ api }) => {
		const plugins = await api.listRepoPlugins(ctx.repositoryId!);
		const found = plugins.find((p) => p.id === pluginRepoId);

		expect(found).toBeDefined();
		expect(found!.gitUrl).toBe(PLUGIN_GIT_URL);
	});

	// --- TASK-01: Create task asking agent about the skill ---------------------

	test("TASK-01: Create task with plugin skill question", async ({ api }) => {
		const { taskId, task } = await createTaskWithMessage(
			api,
			ctx.repositoryId!,
			`Do you have access to a skill called "ad-campaign-best-practices"? ` +
				`Reply with EXACTLY this format on separate lines:\n` +
				`SKILL_CHECK: YES or NO\n` +
				`SKILL_NAME: the exact skill name if found, or NONE`,
		);

		expect(task.status).toBe("open");
		ctx.taskId = taskId;
		console.log(`[Plugin] Task created: ${taskId}`);
	});

	// --- SANDBOX-01: Wait for sandbox ready -----------------------------------

	test("SANDBOX-01: Sandbox provisions with plugin", async ({ api }, testInfo) => {
		testInfo.setTimeout(200000);

		const { ready, status } = await waitForSandboxReady(api, ctx.taskId!);

		expect(ready).toBe(true);
		expect(status!.environmentStatus).toBe("ready");
		console.log("[Plugin] Sandbox ready (plugin repo should be cloned)");
	});

	// --- SANDBOX-01.1: Check for plugin clone event (diagnostic) --------------

	test("SANDBOX-01.1: Check for plugin clone event", async () => {
		// Give events time to arrive
		await new Promise((r) => setTimeout(r, 2000));

		const events = chainSse.getEventsForTask(ctx.taskId!);
		const summary = chainSse.getEventSummary(ctx.taskId!);
		console.log("[Plugin] SSE event summary:", JSON.stringify(summary));

		// Check for PluginReposCloned event
		const cloneEvent = events.find(
			(e) => e.type === "sandbox" && e.payload?.reason === "PluginReposCloned",
		);

		if (cloneEvent) {
			console.log("[Plugin] PluginReposCloned event found — clone succeeded");
		} else {
			console.warn(
				"[Plugin] WARNING: No PluginReposCloned event — sandbox image may not support plugin repo cloning. " +
					"Rebuild sandbox image with latest code: docker/build-sandbox.sh --local",
			);
		}

		// Log all sandbox events for diagnostics
		const sandboxEvents = events.filter((e) => e.type === "sandbox");
		for (const e of sandboxEvents) {
			console.log(`  [SSE] sandbox.${e.subType || "*"} reason=${e.payload?.reason || "?"}`);
		}
	});

	// --- AGENT-01: Wait for agent to complete ---------------------------------

	test("AGENT-01: Agent completes with plugin access", async ({ api }, testInfo) => {
		testInfo.setTimeout(360000);

		const { completed } = await waitForAgentTurnComplete(
			chainSse,
			api,
			ctx.taskId!,
			1,
			300000,
		);

		expect(completed).toBe(true);
		console.log("[Plugin] Agent turn completed");
	});

	// --- AGENT-01.1: Verify agent confirms skill access -----------------------

	test("AGENT-01.1: Agent confirms access to plugin skill", async ({ api }) => {
		const { messages } = await getTaskMessages(api, ctx.taskId!);

		// Log all messages for debugging
		for (const m of messages) {
			const texts = (m.content ?? [])
				.filter((b) => b.type === "text" && !!b.text)
				.map((b) => b.text);
			if (texts.length > 0) {
				console.log(`[Plugin] ${m.role}: ${texts.join(" ").substring(0, 300)}`);
			}
		}

		// Find assistant messages
		const assistantMessages = messages.filter((m) => m.role === "assistant");
		expect(assistantMessages.length).toBeGreaterThanOrEqual(1);

		// Concatenate all assistant message text content
		const fullResponse = assistantMessages
			.flatMap((m) => m.content ?? [])
			.filter((block) => block.type === "text" && !!block.text)
			.map((block) => block.text as string)
			.join("\n");

		// Check for PluginReposCloned event — if missing, the failure is expected
		const events = chainSse.getEventsForTask(ctx.taskId!);
		const hadCloneEvent = events.some(
			(e) => e.type === "sandbox" && e.payload?.reason === "PluginReposCloned",
		);

		if (!hadCloneEvent) {
			console.warn(
				"[Plugin] No PluginReposCloned SSE event — sandbox image likely doesn't support plugin cloning. " +
					"Rebuild with: docker/build-sandbox.sh --local",
			);
		}

		// Verify the agent found the skill
		expect(
			fullResponse,
			"Agent should confirm skill access (SKILL_CHECK: YES). " +
				"If sandbox image is stale, rebuild it to include clone_plugin_repos support.",
		).toContain("SKILL_CHECK: YES");
		expect(fullResponse.toLowerCase()).toContain("ad-campaign-best-practices");
	});
});
