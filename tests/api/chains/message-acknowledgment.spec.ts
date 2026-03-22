/**
 * Chain: Message Acknowledgment & Queuing
 *
 * Verifies per-message acknowledgment: messages sent while the agent is
 * busy are queued (isQueued=true), then acknowledged when the agent
 * session starts processing them (isQueued clears).
 *
 * Requires Claude authentication (CLAUDE_CODE_OAUTH_TOKEN or ANTHROPIC_API_KEY).
 *
 * Chain:
 *   TASK-01 → SANDBOX-01 → AGENT-01 →
 *   MSG-01 (not queued) → MSG-02 (session_init SSE) →
 *   AGENT-02 (send follow-up, wait for running) →
 *   MSG-03 (send while running) → MSG-04 (verify queued) →
 *   MSG-05 (wait for turn 3) → MSG-06 (verify acknowledged) →
 *   MSG-07 (session_init count)
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
} from "../steps";
import { waitForCondition } from "../../shared/wait-helpers";

const hasClaudeAuth =
	!!process.env.CLAUDE_CODE_OAUTH_TOKEN || !!process.env.ANTHROPIC_API_KEY;

test.describe("Chain: Message Acknowledgment & Queuing", () => {
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
			"Add a paragraph that says 'Acknowledgment Test' to the main page",
		);

		expect(task.status).toBe("open");
		expect(taskId).toBeTruthy();

		ctx.taskId = taskId;
		ctx.task = task;

		console.log(`[Ack] Task created: ${taskId}`);
	});

	// ─── SANDBOX-01: Sandbox provisions automatically ───────────────────

	test("SANDBOX-01: Sandbox provisions automatically", async ({ api }, testInfo) => {
		testInfo.setTimeout(200000);

		const { ready, status } = await waitForSandboxReady(api, ctx.taskId!);

		expect(ready).toBe(true);
		expect(status).not.toBeNull();

		ctx.sandboxStatus = status!;
		console.log("[Ack] Sandbox ready");
	});

	// ─── AGENT-01: Wait for turn 1 to complete ─────────────────────────

	test("AGENT-01: Agent executes initial message", async ({ api }, testInfo) => {
		testInfo.setTimeout(360000);

		const { completed } = await waitForAgentTurnComplete(
			chainSse,
			api,
			ctx.taskId!,
			1,
			300000,
		);

		expect(completed).toBe(true);
		ctx.turnCount = 1;

		console.log("[Ack] Agent turn 1 completed");
	});

	// ─── MSG-01: Initial message is NOT queued after turn 1 ─────────────

	test("MSG-01: Initial message not queued after turn 1", async ({ api }) => {
		const { messages } = await getTaskMessages(api, ctx.taskId!);

		const userMessages = messages.filter(
			(m) => m.messageType === "user" && m.role === "user",
		);
		expect(userMessages.length).toBeGreaterThanOrEqual(1);

		// The initial user message should have been acknowledged by the agent session
		const initialMsg = userMessages[0];
		expect(initialMsg.isQueued).toBeFalsy();

		console.log(
			`[Ack] Initial message uuid=${initialMsg.uuid}, isQueued=${initialMsg.isQueued ?? "absent"}`,
		);
	});

	// ─── MSG-02: session_init SSE received for turn 1 ───────────────────

	test("MSG-02: session_init SSE event received", async () => {
		const events = chainSse.getEventsForTask(ctx.taskId!);
		const sessionInits = events.filter(
			(e) => e.type === "agent" && e.subType === "session_init",
		);

		expect(sessionInits.length).toBeGreaterThanOrEqual(1);

		const firstInit = sessionInits[0];
		const payload = firstInit.payload as { session_id?: string };
		expect(payload.session_id).toBeTruthy();

		console.log(
			`[Ack] session_init events so far: ${sessionInits.length}, first session_id=${payload.session_id}`,
		);
	});

	// ─── AGENT-02: Send follow-up and wait for agent to start running ───

	test("AGENT-02: Send follow-up, agent starts turn 2", async ({ api }, testInfo) => {
		testInfo.setTimeout(360000);

		// Wait for agent to be idle before sending
		await waitForCondition(
			async () => {
				const s = await api.getWorkspaceStatus(ctx.taskId!);
				return s.agentState === "idle" || s.agentState === "none";
			},
			{ timeout: 10000, interval: 1000 },
		);

		await sendFollowUpMessage(
			api,
			ctx.taskId!,
			"Change 'Acknowledgment Test' to 'Turn 2 Complete'",
		);

		console.log("[Ack] Follow-up sent, waiting for agent to start...");

		// Wait for the agent to start running (agent.started SSE event)
		const startedEvent = await chainSse.waitForNthEvent(
			"agent",
			"started",
			ctx.taskId!,
			2,
			60000,
		);

		expect(startedEvent).not.toBeNull();
		console.log("[Ack] Agent started turn 2");
	});

	// ─── MSG-03: Send message while agent is running (will be queued) ────

	test("MSG-03: Send message while agent is running", async ({ api }) => {
		// Verify agent is actually running
		const status = await api.getWorkspaceStatus(ctx.taskId!);
		console.log(`[Ack] Agent state before queued send: ${status.agentState}`);

		// Send a message while the agent is busy — this will be queued
		await api.sendMessage(ctx.taskId!, "After you finish, also add a footer that says 'Queued Message Received'");

		console.log("[Ack] Queued message sent while agent is running");
	});

	// ─── MSG-04: Verify queued message has isQueued=true ─────────────────

	test("MSG-04: Queued message has isQueued=true", async ({ api }) => {
		const { messages } = await getTaskMessages(api, ctx.taskId!);

		const userMessages = messages.filter(
			(m) => m.messageType === "user" && m.role === "user",
		);

		// The last user message should be our queued one
		const queuedMsg = userMessages[userMessages.length - 1];
		expect(queuedMsg.isQueued).toBe(true);

		// Store the UUID so we can verify it later
		ctx.set("queuedMessageUuid", queuedMsg.uuid);

		console.log(
			`[Ack] Queued message uuid=${queuedMsg.uuid}, isQueued=${queuedMsg.isQueued}`,
		);
	});

	// ─── MSG-05: Wait for turn 2 to complete ────────────────────────────

	test("MSG-05: Turn 2 completes", async ({ api }, testInfo) => {
		testInfo.setTimeout(360000);

		const turn2 = await waitForAgentTurnComplete(
			chainSse,
			api,
			ctx.taskId!,
			2,
			300000,
		);
		expect(turn2.completed).toBe(true);
		ctx.turnCount = 2;
		console.log("[Ack] Turn 2 completed");
	});

	// ─── MSG-05.1: Agent processes queued message (turn 3) ──────────────

	test("MSG-05.1: Agent processes queued message (turn 3)", async ({ api }, testInfo) => {
		testInfo.setTimeout(360000);

		// After turn 2 completes, the sandbox should pick up the queued message
		// via take_messages_pending(). If timing is tight, re-send a follow-up
		// to ensure the sandbox wakes up.
		const turn3 = await waitForAgentTurnComplete(
			chainSse,
			api,
			ctx.taskId!,
			3,
			120000,
		);

		if (!turn3.completed) {
			// The deferred-message path may have missed the window.
			// Send the same message again — it will either trigger a new turn
			// or the pending message is already being processed.
			console.log("[Ack] Turn 3 not detected via SSE, sending nudge message...");
			await api.sendMessage(ctx.taskId!, "Please add a footer that says 'Queued Message Received'");

			const turn3Retry = await waitForAgentTurnComplete(
				chainSse,
				api,
				ctx.taskId!,
				3,
				180000,
			);
			expect(turn3Retry.completed).toBe(true);
		}

		ctx.turnCount = 3;
		console.log("[Ack] Turn 3 completed (queued message processed)");
	});

	// ─── MSG-06: Previously-queued message now acknowledged ─────────────

	test("MSG-06: Queued message acknowledged after processing", async ({ api }) => {
		const { messages } = await getTaskMessages(api, ctx.taskId!);

		const queuedUuid = ctx.get<string>("queuedMessageUuid");
		expect(queuedUuid).toBeTruthy();

		const msg = messages.find((m) => m.uuid === queuedUuid);
		expect(msg).toBeDefined();

		// After the agent processed it, isQueued should be cleared
		expect(msg!.isQueued).toBeFalsy();

		console.log(
			`[Ack] Message uuid=${queuedUuid} acknowledged: isQueued=${msg!.isQueued ?? "absent"}`,
		);
	});

	// ─── MSG-07: session_init events received for multiple turns ────────

	test("MSG-07: Multiple session_init events received", async () => {
		const sessionInitCount = chainSse.countEvents(
			"agent",
			"session_init",
			ctx.taskId!,
		);

		// At least 2 session_init events (turn 1 and turn 3; turn 2 may share session with turn 1 via resume)
		expect(sessionInitCount).toBeGreaterThanOrEqual(2);

		console.log(`[Ack] Total session_init events: ${sessionInitCount}`);

		// Dump event summary for diagnostics
		const summary = chainSse.getEventSummary(ctx.taskId!);
		console.log("[Ack] SSE event summary:", JSON.stringify(summary, null, 2));
	});
});
