/**
 * Chain 3: Agent Stop, Resume & Recovery (agent-execution.md)
 *
 * Stopping an agent mid-execution, resuming after a sandbox restart,
 * and recovering from agent errors.
 *
 * Requires Claude authentication (CLAUDE_CODE_OAUTH_TOKEN or ANTHROPIC_API_KEY).
 *
 * Chain:
 *   TASK-01 → AGENT-04 → AGENT-04.1 → AGENT-04.2 →
 *   AGENT-06.1 → AGENT-06.2 → AGENT-06.3 → AGENT-06.4 → AGENT-06.5 →
 *   AGENT-05 (skipped) → AGENT-05.1 (skipped) → AGENT-05.2 (skipped)
 */

import { test, expect } from "../fixtures/api-fixtures";
import { getBaseURL } from "../fixtures/api-fixtures";
import { ChainContext } from "../context/chain-context";
import { SSEEventCollector } from "../../shared/sse-collector";
import {
	createTaskWithMessage,
	waitForSandboxReady,
	stopAgent,
	stopSandbox,
	startSandbox,
	listFiles,
	getTaskMessages,
	sendFollowUpMessage,
	findFileWithExpectedContent,
} from "../steps";

const hasClaudeAuth =
	!!process.env.CLAUDE_CODE_OAUTH_TOKEN || !!process.env.ANTHROPIC_API_KEY;

test.describe("Chain: Agent Stop, Resume & Recovery", () => {
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

	// ─── TASK-01: Create task with message (agent starts) ───────────────

	test("TASK-01: Create task with message (agent starts)", async ({ api }, testInfo) => {
		testInfo.setTimeout(200000);

		const { taskId, task } = await createTaskWithMessage(
			api,
			ctx.repositoryId!,
			"Create a complex multi-page application with a home page, about page, and contact page. Take your time to plan this carefully.",
		);

		expect(task.status).toBe("open");
		ctx.taskId = taskId;
		ctx.task = task;

		console.log(`[Stop] Task created: ${taskId}`);

		// Wait for sandbox to be ready
		const { ready } = await waitForSandboxReady(api, taskId);
		expect(ready).toBe(true);
		console.log("[Stop] Sandbox ready");

		// Verify sandbox SSE events arrived
		const sandboxEvents = chainSse.getEventsForTask(taskId)
			.filter((e) => e.type === "sandbox" || e.type === "environment");
		console.log(`[Stop] Sandbox SSE events: ${sandboxEvents.length}`);
		for (const e of sandboxEvents) {
			console.log(`  [SSE] ${e.type}.${e.subType || "*"}`);
		}

		// Check workspace status for agent state
		const wsStatus = await api.getWorkspaceStatus(taskId);
		console.log(`[Stop] Workspace: env=${wsStatus.environmentStatus}, agent=${wsStatus.agentState}`);

		// Wait for agent to start running
		const agentStarted = await chainSse.waitForEvent({
			type: "agent",
			subType: "started",
			taskId,
			timeout: 120000,
		});

		if (!agentStarted) {
			// Diagnostic: dump all events received
			const summary = chainSse.getEventSummary(taskId);
			console.log("[Stop] SSE event summary:", JSON.stringify(summary, null, 2));

			const allEvents = chainSse.getEventsForTask(taskId);
			console.log(`[Stop] Total events for task: ${allEvents.length}`);
			for (const e of allEvents) {
				console.log(`  [SSE] ${e.type}.${e.subType || "*"} @ ${e.timestamp || "?"}`);
			}

			// Also check if agent is already running via API
			const currentStatus = await api.getWorkspaceStatus(taskId);
			console.log(`[Stop] Current agent state via API: ${currentStatus.agentState}`);
		}

		expect(agentStarted).not.toBeNull();
		console.log("[Stop] Agent started running");
	});

	// ─── AGENT-04: Stop agent mid-execution ─────────────────────────────

	test("AGENT-04: Stop agent mid-execution", async ({ api }, testInfo) => {
		testInfo.setTimeout(60000);

		// Stop the running agent
		await stopAgent(api, ctx.taskId!);

		// Wait for agent to become idle
		const deadline = Date.now() + 30000;
		let isIdle = false;

		while (Date.now() < deadline) {
			const status = await api.getWorkspaceStatus(ctx.taskId!);
			if (status.agentState === "idle" || status.agentState === "none") {
				isIdle = true;
				break;
			}
			await new Promise((r) => setTimeout(r, 1000));
		}

		expect(isIdle).toBe(true);
		console.log("[Stop] Agent stopped");
	});

	// ─── AGENT-04.1: Sandbox remains ready ──────────────────────────────

	test("AGENT-04.1: Sandbox remains ready", async ({ api }) => {
		const status = await api.getWorkspaceStatus(ctx.taskId!);

		expect(status.environmentStatus).toBe("ready");
		console.log("[Stop] Sandbox still ready after agent stop");
	});

	// ─── AGENT-04.2: SSE agent.stopped event ────────────────────────────

	test("AGENT-04.2: SSE agent.stopped event", async () => {
		const events = chainSse.getEventsForTask(ctx.taskId!);

		const hasAgentStopped = events.some(
			(e) => e.type === "agent" && e.subType === "stopped",
		);
		const hasTurnStopped = events.some(
			(e) => e.type === "taskTurn" && e.subType === "stopped",
		);

		// At least one of the stop events should be present
		expect(hasAgentStopped || hasTurnStopped).toBe(true);

		console.log(`[Stop] Events: agent.stopped=${hasAgentStopped}, taskTurn.stopped=${hasTurnStopped}`);
	});

	// ─── AGENT-06.1: Sandbox stops ──────────────────────────────────────

	test("AGENT-06.1: Sandbox stops", async ({ api }, testInfo) => {
		testInfo.setTimeout(90000);

		const status = await stopSandbox(api, ctx.taskId!);

		expect(status).not.toBeNull();
		expect(status!.environmentStatus).toBe("stopped");

		console.log("[Stop] Sandbox stopped");
	});

	// ─── AGENT-06.2: Sandbox restarts with files intact ─────────────────

	test("AGENT-06.2: Sandbox restarts with files intact", async ({ api }, testInfo) => {
		testInfo.setTimeout(200000);

		const { ready } = await startSandbox(api, ctx.taskId!);
		expect(ready).toBe(true);

		const files = await listFiles(api, ctx.taskId!, ".", false);
		expect(files.length).toBeGreaterThan(0);

		console.log(`[Stop] Sandbox restarted, ${files.length} files present`);
	});

	// ─── AGENT-06.3: Agent resumes with follow-up message ───────────────

	test("AGENT-06.3: Agent resumes with follow-up message", async ({ api }, testInfo) => {
		testInfo.setTimeout(360000);

		const turnsBefore = chainSse.countEvents("taskTurn", "completed", ctx.taskId!);

		await sendFollowUpMessage(
			api,
			ctx.taskId!,
			"Add a paragraph that says 'Resumed Session Works'",
		);

		console.log("[Stop] Follow-up message sent after restart");

		const event = await chainSse.waitForNthEvent(
			"taskTurn",
			"completed",
			ctx.taskId!,
			turnsBefore + 1,
			300000,
		);

		expect(event).not.toBeNull();
		console.log("[Stop] Agent completed resumed turn");
	});

	// ─── AGENT-06.4: Agent retains conversation history ─────────────────

	test("AGENT-06.4: Agent retains conversation history", async ({ api }) => {
		const { messages } = await getTaskMessages(api, ctx.taskId!);

		// Should have at least: original user msg, agent response, follow-up msg, agent response
		expect(messages.length).toBeGreaterThanOrEqual(4);

		const hasUser = messages.some((m) => m.role === "user");
		const hasAssistant = messages.some((m) => m.role === "assistant");
		expect(hasUser).toBe(true);
		expect(hasAssistant).toBe(true);

		// Verify messages from both before and after restart
		const userMessages = messages.filter((m) => m.role === "user");
		expect(userMessages.length).toBeGreaterThanOrEqual(2);

		console.log(`[Stop] Conversation history: ${messages.length} messages, ${userMessages.length} user messages`);
	});

	// ─── AGENT-06.5: Files reflect resumed-session changes ──────────────

	test("AGENT-06.5: Files reflect resumed-session changes", async ({ api }) => {
		const match = await findFileWithExpectedContent(api, ctx.taskId!, "Resumed Session Works");

		expect(match).not.toBeNull();
		console.log(`[Stop] Content "Resumed Session Works" found in: ${match!.path}`);
	});

	// ─── AGENT-05: Agent error sets task to error (skipped) ─────────────

	test.skip("AGENT-05: Agent error sets task to error", async ({ api }, testInfo) => {
		testInfo.setTimeout(120000);

		// This test requires a way to trigger an agent error.
		// Currently skipped — requires the ability to inject an error-causing message.
		//
		// When implemented:
		// 1. Send a message that triggers an agent error
		// 2. Wait for agent.error and taskTurn.error SSE events
		// 3. Verify agentState returns to idle

		expect(true).toBe(true);
	});

	// ─── AGENT-05.1: Error visible in chat (skipped) ────────────────────

	test.skip("AGENT-05.1: Error visible in chat", async ({ api }) => {
		// Depends on AGENT-05
		const { messages } = await getTaskMessages(api, ctx.taskId!);
		expect(messages.length).toBeGreaterThan(0);
	});

	// ─── AGENT-05.2: Recovery via new message (skipped) ─────────────────

	test.skip("AGENT-05.2: Recovery via new message", async ({ api }, testInfo) => {
		testInfo.setTimeout(120000);

		// Depends on AGENT-05 which is skipped
		await sendFollowUpMessage(api, ctx.taskId!, "Just add a simple heading to the home page");

		const turnActive = await chainSse.waitForEvent({
			type: "taskTurn",
			subType: "active",
			taskId: ctx.taskId,
			timeout: 60000,
		});

		expect(turnActive).not.toBeNull();

		const status = await api.getWorkspaceStatus(ctx.taskId!);
		expect(["running", "idle"]).toContain(status.agentState);
	});
});
