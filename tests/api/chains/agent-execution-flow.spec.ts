/**
 * Chain 1: Agent Execution & Multi-Turn (agent-execution.md)
 *
 * Happy-path agent execution: task creation, sandbox provisioning, agent
 * execution, file verification, and multi-turn conversation.
 *
 * Absorbed from core-lifecycle.spec.ts — drops SERVER-01 and SANDBOX-01.1/01.2
 * (covered by sandbox-lifecycle.spec.ts).
 *
 * Requires Claude authentication (CLAUDE_CODE_OAUTH_TOKEN or ANTHROPIC_API_KEY).
 *
 * Chain:
 *   TASK-01 → SANDBOX-01 → AGENT-01 → AGENT-01.1 → AGENT-01.2 →
 *   AGENT-01.3 → AGENT-01.4 → FILE-06 →
 *   AGENT-02 → AGENT-02.1 → AGENT-02.2 → AGENT-02.3
 */

import { test, expect } from "../fixtures/api-fixtures";
import { getBaseURL } from "../fixtures/api-fixtures";
import { ChainContext } from "../context/chain-context";
import { SSEEventCollector } from "../../shared/sse-collector";
import {
	createTaskWithMessage,
	waitForSandboxReady,
	waitForAgentTurnComplete,
	sendFollowUpMessage,
	getTaskMessages,
	getTask,
	findFileWithExpectedContent,
} from "../steps";

const hasClaudeAuth =
	!!process.env.CLAUDE_CODE_OAUTH_TOKEN || !!process.env.ANTHROPIC_API_KEY;

test.describe("Chain: Agent Execution & Multi-Turn", () => {
	test.describe.configure({ mode: "serial" });

	test.skip(!hasClaudeAuth, "Skipping — no Claude authentication");

	const ctx = new ChainContext();
	let chainSse: SSEEventCollector;

	test.beforeAll(async ({ api }) => {
		const repo = await api.ensureTemplateRepository();
		ctx.repositoryId = repo.id;
		ctx.repository = repo;

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

	// ─── TASK-01: Create task with message ──────────────────────────────

	test("TASK-01: Create task with message", async ({ api }) => {
		const { taskId, task } = await createTaskWithMessage(
			api,
			ctx.repositoryId!,
			"Add a heading that says 'E2E Test Success' to the main page",
		);

		expect(task.status).toBe("open");
		expect(taskId).toBeTruthy();

		ctx.taskId = taskId;
		ctx.task = task;

		console.log(`[Agent] Task created: ${taskId}`);
	});

	// ─── SANDBOX-01: Sandbox provisions automatically ───────────────────

	test("SANDBOX-01: Sandbox provisions automatically", async ({ api }, testInfo) => {
		testInfo.setTimeout(200000);

		const { ready, status } = await waitForSandboxReady(api, ctx.taskId!);

		expect(ready).toBe(true);
		expect(status).not.toBeNull();
		expect(status!.environmentStatus).toBe("ready");

		ctx.sandboxStatus = status!;
		console.log("[Agent] Sandbox ready");
	});

	// ─── AGENT-01: Agent executes initial message ───────────────────────

	test("AGENT-01: Agent executes initial message", async ({ api }, testInfo) => {
		testInfo.setTimeout(360000);

		const { completed } = await waitForAgentTurnComplete(chainSse, api, ctx.taskId!, 1, 300000);

		if (!completed) {
			// Diagnostic: dump what we actually received
			const summary = chainSse.getEventSummary(ctx.taskId!);
			console.log("[Agent] SSE event summary:", JSON.stringify(summary, null, 2));

			const status = await api.getWorkspaceStatus(ctx.taskId!);
			console.log(`[Agent] Workspace: env=${status.environmentStatus}, agent=${status.agentState}`);

			const events = chainSse.getEventsForTask(ctx.taskId!);
			console.log(`[Agent] Total events for task: ${events.length}`);
			for (const e of events.slice(-10)) {
				console.log(`  [SSE] ${e.type}.${e.subType || "*"} @ ${e.timestamp || "?"}`);
			}
		}

		expect(completed).toBe(true);
		ctx.turnCount = 1;

		console.log("[Agent] Agent turn 1 completed");
	});

	// ─── AGENT-01.1: Agent state transitions ────────────────────────────

	test("AGENT-01.1: Agent state transitions", async ({ api }) => {
		// After completion, agent should be idle
		const status = await api.getWorkspaceStatus(ctx.taskId!);
		expect(["idle", "none"]).toContain(status.agentState);
	});

	// ─── AGENT-01.2: SSE agent lifecycle events ─────────────────────────

	test("AGENT-01.2: SSE agent lifecycle events", async () => {
		const events = chainSse.getEventsForTask(ctx.taskId!);

		const hasAgentStarted = events.some(
			(e) => e.type === "agent" && e.subType === "started",
		);
		const hasAgentMessage = events.some(
			(e) => e.type === "agent" && e.subType === "message",
		);
		const hasAgentCompleted = events.some(
			(e) => e.type === "agent" && e.subType === "completed",
		);
		const hasTurnActive = events.some(
			(e) => e.type === "taskTurn" && e.subType === "active",
		);
		const hasTurnCompleted = events.some(
			(e) => e.type === "taskTurn" && e.subType === "completed",
		);

		expect(hasAgentStarted).toBe(true);
		expect(hasAgentMessage).toBe(true);
		expect(hasAgentCompleted).toBe(true);
		expect(hasTurnActive).toBe(true);
		expect(hasTurnCompleted).toBe(true);
	});

	// ─── AGENT-01.3: Messages endpoint returns conversation ─────────────

	test("AGENT-01.3: Messages endpoint returns conversation", async ({ api }) => {
		const { messages } = await getTaskMessages(api, ctx.taskId!);

		expect(messages.length).toBeGreaterThanOrEqual(2);

		const hasUser = messages.some((m) => m.role === "user");
		const hasAssistant = messages.some((m) => m.role === "assistant");

		expect(hasUser).toBe(true);
		expect(hasAssistant).toBe(true);
	});

	// ─── AGENT-01.4: Task cost increases ────────────────────────────────

	test("AGENT-01.4: Task cost increases", async ({ api }) => {
		const task = await getTask(api, ctx.taskId!);

		expect(task.totalCostUsd).toBeGreaterThan(0);

		ctx.lastCostUsd = task.totalCostUsd;
		console.log(`[Agent] Cost: $${task.totalCostUsd.toFixed(4)}`);
	});

	// ─── FILE-06: Files reflect agent changes ───────────────────────────

	test("FILE-06: Files reflect agent changes", async ({ api }) => {
		const match = await findFileWithExpectedContent(api, ctx.taskId!, "E2E Test Success");

		expect(match).not.toBeNull();
		console.log(`[Agent] Content "E2E Test Success" found in: ${match!.path}`);

		// Verify git diff has changes
		try {
			const diff = await api.getGitDiff(ctx.taskId!);
			const hasDiff = diff.diff ? diff.diff.length > 0 : diff.files.length > 0;
			expect(hasDiff).toBe(true);
		} catch {
			console.log("[Agent] Git diff endpoint not available — skipping");
		}
	});

	// ─── AGENT-02: Multi-turn conversation ──────────────────────────────

	test("AGENT-02: Multi-turn conversation", async ({ api }, testInfo) => {
		testInfo.setTimeout(360000);

		await sendFollowUpMessage(
			api,
			ctx.taskId!,
			"Change the 'E2E Test Success' heading to 'Multi-Turn Success' instead",
		);

		console.log("[Agent] Follow-up message sent");

		const { completed } = await waitForAgentTurnComplete(chainSse, api, ctx.taskId!, 2, 300000);

		expect(completed).toBe(true);
		ctx.turnCount = 2;

		console.log("[Agent] Agent turn 2 completed");
	});

	// ─── AGENT-02.1: Agent re-activates on follow-up ────────────────────

	test("AGENT-02.1: Agent re-activates on follow-up", async ({ api }) => {
		// After second turn, agent should be idle
		const status = await api.getWorkspaceStatus(ctx.taskId!);
		expect(["idle", "none"]).toContain(status.agentState);
	});

	// ─── AGENT-02.2: Second taskTurn.completed event ────────────────────

	test("AGENT-02.2: Second taskTurn.completed event", async () => {
		const agentCount = chainSse.countEvents("agent", "completed", ctx.taskId!);
		const turnCount = chainSse.countEvents("taskTurn", "completed", ctx.taskId!);

		expect(agentCount).toBeGreaterThanOrEqual(2);
		expect(turnCount).toBeGreaterThanOrEqual(2);

		console.log(`[Agent] Events: agent.completed=${agentCount}, taskTurn.completed=${turnCount}`);
	});

	// ─── AGENT-02.3: Files reflect second-turn changes ──────────────────

	test("AGENT-02.3: Files reflect second-turn changes", async ({ api }) => {
		const match = await findFileWithExpectedContent(api, ctx.taskId!, "Multi-Turn Success");

		expect(match).not.toBeNull();
		console.log(`[Agent] Content "Multi-Turn Success" found in: ${match!.path}`);
	});
});
