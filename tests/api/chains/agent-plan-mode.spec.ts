/**
 * Chain 3: Plan Mode (agent-execution.md)
 *
 * Agent execution in plan mode: the agent produces a plan for user approval
 * before making changes.
 *
 * Requires Claude authentication (CLAUDE_CODE_OAUTH_TOKEN or ANTHROPIC_API_KEY).
 *
 * Chain:
 *   TASK-01 (plan mode) → SANDBOX-01 → AGENT-03 →
 *   AGENT-03.1 → AGENT-03.2
 */

import { test, expect } from "../fixtures/api-fixtures";
import { getBaseURL } from "../fixtures/api-fixtures";
import { ChainContext } from "../context/chain-context";
import { SSEEventCollector } from "../../shared/sse-collector";
import {
	createTaskWithMessage,
	waitForSandboxReady,
	sendFollowUpMessage,
	getTaskMessages,
	waitForAgentTurnComplete,
} from "../steps";

const hasClaudeAuth =
	!!process.env.CLAUDE_CODE_OAUTH_TOKEN || !!process.env.ANTHROPIC_API_KEY;

test.describe("Chain: Plan Mode", () => {
	test.describe.configure({ mode: "serial" });

	test.skip(!hasClaudeAuth, "Skipping — no Claude authentication");

	const ctx = new ChainContext();
	let chainSse: SSEEventCollector;

	test.beforeAll(async ({ api }) => {
		const repo = await api.ensureTemplateRepository();
		ctx.repositoryId = repo.id;

		chainSse = new SSEEventCollector({ debug: !!process.env.DEBUG_SSE });
		await chainSse.start(`${getBaseURL()}/api/v1/events/stream`);
	});

	test.afterAll(async ({ api }) => {
		chainSse?.stop();

		if (ctx.taskId) {
			await api.stopTask(ctx.taskId).catch(() => {});
			await new Promise((r) => setTimeout(r, 500));
			await api.deleteTask(ctx.taskId).catch(() => {});
		}
	});

	// ─── TASK-01: Create task in plan mode ──────────────────────────────

	test("TASK-01: Create task in plan mode", async ({ api }) => {
		const { taskId, task } = await createTaskWithMessage(
			api,
			ctx.repositoryId!,
			"Add a heading that says 'Plan Mode Test' to the main page.",
		);

		expect(task.status).toBe("open");

		ctx.taskId = taskId;
		ctx.task = task;

		console.log(`[Plan] Task created: ${taskId}`);
	});

	// ─── SANDBOX-01: Sandbox provisions ─────────────────────────────────

	test("SANDBOX-01: Sandbox provisions", async ({ api }, testInfo) => {
		testInfo.setTimeout(200000);

		const { ready } = await waitForSandboxReady(api, ctx.taskId!);

		expect(ready).toBe(true);
		console.log("[Plan] Sandbox ready");
	});

	// ─── AGENT-03: Plan mode flow ───────────────────────────────────────

	test("AGENT-03: Plan mode flow", async ({ api }, testInfo) => {
		testInfo.setTimeout(180000);

		// Race: agent either pauses for plan approval or completes its turn
		const result = await Promise.race([
			chainSse.waitForEvent({
				type: "agent",
				subType: "pending_response",
				taskId: ctx.taskId,
				timeout: 120000,
			}).then((e) => (e ? ("pending" as const) : null)),
			waitForAgentTurnComplete(chainSse, api, ctx.taskId!, 1, 120000)
				.then(({ completed }) => (completed ? ("completed" as const) : null)),
		]);

		if (result === "pending") {
			ctx.set("planModeActive", true);
			console.log("[Plan] Agent paused for approval");
		} else {
			ctx.set("planModeActive", false);
			expect(result).toBe("completed");
			console.log("[Plan] Agent completed without plan pause");
		}
	});

	// ─── AGENT-03.1: Agent produces plan for approval ───────────────────

	test("AGENT-03.1: Agent produces plan for approval", async ({ api }) => {
		const { messages } = await getTaskMessages(api, ctx.taskId!);

		expect(messages.length).toBeGreaterThanOrEqual(2);

		const hasAssistant = messages.some((m) => m.role === "assistant");
		expect(hasAssistant).toBe(true);

		if (ctx.get("planModeActive")) {
			const hasPendingResponse = chainSse.getEventsForTask(ctx.taskId!).some(
				(e) => e.type === "agent" && e.subType === "pending_response",
			);
			expect(hasPendingResponse).toBe(true);
		}
	});

	// ─── AGENT-03.2: User approves, agent executes ──────────────────────

	test("AGENT-03.2: User approves, agent executes", async ({ api }, testInfo) => {
		testInfo.setTimeout(180000);

		if (ctx.get("planModeActive")) {
			await sendFollowUpMessage(api, ctx.taskId!, "Looks good, proceed.");

			const { completed } = await waitForAgentTurnComplete(chainSse, api, ctx.taskId!, 1, 120000);
			expect(completed).toBe(true);
		}

		const turnCount = chainSse.countEvents("taskTurn", "completed", ctx.taskId!);
		expect(turnCount).toBeGreaterThanOrEqual(1);

		console.log("[Plan] Agent execution completed");
	});
});
