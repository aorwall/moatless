/**
 * Chain: Message Acknowledgment — Same-Session Multi-Message
 *
 * Verifies that multiple messages sent while the agent is running are
 * injected into the same CLI session (via send_user_message with
 * MessagePriority::Next), not queued for separate turns.
 *
 * Key assertion: only 1 session_init event — all messages handled
 * in a single session.
 *
 * Requires Claude authentication (CLAUDE_CODE_OAUTH_TOKEN or ANTHROPIC_API_KEY).
 *
 * Chain:
 *   TASK-01 → SANDBOX-01 → MSG-01 (session_init) →
 *   MSG-02 (send 2nd) → MSG-03 (send 3rd) →
 *   MSG-04 (wait idle) → MSG-05 (single session) →
 *   MSG-06 (verify files) → MSG-07 (all acknowledged)
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
	findFileWithExpectedContent,
} from "../steps";
import { waitForCondition } from "../../shared/wait-helpers";

const hasClaudeAuth =
	!!process.env.CLAUDE_CODE_OAUTH_TOKEN || !!process.env.ANTHROPIC_API_KEY;

test.describe("Chain: Message Acknowledgment — Same-Session Multi-Message", () => {
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

	// ─── TASK-01: Create task with first message ────────────────────────

	test("TASK-01: Create task with first message", async ({ api }) => {
		const { taskId, task } = await createTaskWithMessage(
			api,
			ctx.repositoryId!,
			"Create a file called first.txt with the content 'First Message Created'",
		);

		expect(task.status).toBe("open");
		expect(taskId).toBeTruthy();

		ctx.taskId = taskId;
		ctx.task = task;

		console.log(`[MultiMsg] Task created: ${taskId}`);
	});

	// ─── SANDBOX-01: Sandbox provisions automatically ───────────────────

	test("SANDBOX-01: Sandbox provisions automatically", async ({ api }, testInfo) => {
		testInfo.setTimeout(200000);

		const { ready, status } = await waitForSandboxReady(api, ctx.taskId!);

		expect(ready).toBe(true);
		expect(status).not.toBeNull();

		ctx.sandboxStatus = status!;
		console.log("[MultiMsg] Sandbox ready");
	});

	// ─── MSG-01: Wait for session_init (session is alive) ───────────────

	test("MSG-01: Session starts for first message", async ({}, testInfo) => {
		testInfo.setTimeout(130000);

		const sessionInit = await chainSse.waitForEvent({
			type: "agent",
			subType: "session_init",
			taskId: ctx.taskId!,
			timeout: 120000,
		});

		expect(sessionInit).not.toBeNull();

		const payload = sessionInit!.payload as { session_id?: string };
		expect(payload.session_id).toBeTruthy();

		ctx.set("sessionId", payload.session_id);
		console.log(`[MultiMsg] session_init received, session_id=${payload.session_id}`);
	});

	// ─── MSG-02: Send second message while agent is running ─────────────

	test("MSG-02: Send second message during active session", async ({ api }) => {
		await sendFollowUpMessage(
			api,
			ctx.taskId!,
			"Also create a file called second.txt with the content 'Second Message Created'",
		);

		console.log("[MultiMsg] Second message sent while agent is running");
	});

	// ─── MSG-03: Send third message after brief delay ───────────────────

	test("MSG-03: Send third message during active session", async ({ api }) => {
		// Brief delay to space out the messages
		await new Promise((r) => setTimeout(r, 2000));

		await sendFollowUpMessage(
			api,
			ctx.taskId!,
			"Also create a file called third.txt with the content 'Third Message Created'",
		);

		console.log("[MultiMsg] Third message sent while agent is running");
	});

	// ─── MSG-04: Wait for agent to become idle ──────────────────────────

	test("MSG-04: Agent completes and becomes idle", async ({ api }, testInfo) => {
		testInfo.setTimeout(360000);

		const settled = await waitForCondition(
			async () => {
				const s = await api.getWorkspaceStatus(ctx.taskId!);
				return s.agentState === "idle" || s.agentState === "none";
			},
			{ timeout: 300000, interval: 2000 },
		);

		expect(settled).toBe(true);

		const status = await api.getWorkspaceStatus(ctx.taskId!);
		console.log(`[MultiMsg] Agent state: ${status.agentState}`);
	});

	// ─── MSG-05: Assert single session (1 session_init) ─────────────────

	test("MSG-05: All messages handled in single session", async () => {
		const sessionInitCount = chainSse.countEvents(
			"agent",
			"session_init",
			ctx.taskId!,
		);

		expect(sessionInitCount).toBe(1);

		console.log(`[MultiMsg] session_init count: ${sessionInitCount} (single session confirmed)`);
	});

	// ─── MSG-06: Verify all three files were created ────────────────────

	test("MSG-06: First message file exists", async ({ api }) => {
		const match = await findFileWithExpectedContent(
			api,
			ctx.taskId!,
			"First Message Created",
		);

		expect(match).not.toBeNull();
		console.log(`[MultiMsg] first.txt found in: ${match!.path}`);
	});

	test("MSG-06.1: Second message file exists", async ({ api }) => {
		const match = await findFileWithExpectedContent(
			api,
			ctx.taskId!,
			"Second Message Created",
		);

		expect(match).not.toBeNull();
		console.log(`[MultiMsg] second.txt found in: ${match!.path}`);
	});

	test("MSG-06.2: Third message file exists", async ({ api }) => {
		const match = await findFileWithExpectedContent(
			api,
			ctx.taskId!,
			"Third Message Created",
		);

		expect(match).not.toBeNull();
		console.log(`[MultiMsg] third.txt found in: ${match!.path}`);
	});

	// ─── MSG-07: All messages acknowledged ──────────────────────────────

	test("MSG-07: All messages acknowledged", async ({ api }) => {
		const { messages } = await getTaskMessages(api, ctx.taskId!);

		const queuedMessages = messages.filter((m) => m.isQueued === true);

		expect(queuedMessages.length).toBe(0);

		console.log(
			`[MultiMsg] All ${messages.length} messages acknowledged, 0 queued`,
		);

		// Dump event summary
		const summary = chainSse.getEventSummary(ctx.taskId!);
		console.log("[MultiMsg] SSE event summary:", JSON.stringify(summary, null, 2));
	});
});
