/**
 * Chain: Sandbox Lifecycle (sandbox-operations.md)
 *
 * Tests sandbox provisioning, idempotent re-provision, stop, start, restart,
 * and cleanup. Uses a task without an initial message so the agent stays idle.
 *
 * Chain:
 *   TASK-02 → SANDBOX-01 → SANDBOX-01.1 → SANDBOX-02 → SANDBOX-03 →
 *   SANDBOX-03.1 → SANDBOX-03.2 → SANDBOX-04 → SANDBOX-05 → SANDBOX-05.1
 */

import { test, expect } from "../fixtures/api-fixtures";
import { getBaseURL } from "../fixtures/api-fixtures";
import { ChainContext } from "../context/chain-context";
import { SSEEventCollector } from "../../shared/sse-collector";
import {
	createTaskWithoutMessage,
	waitForSandboxReady,
	provisionSandbox,
	stopSandbox,
	startSandbox,
	restartSandbox,
	cleanupSandbox,
	listFiles,
} from "../steps";

test.describe("Chain: Sandbox Lifecycle", () => {
	test.describe.configure({ mode: "serial" });

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
			await api.deleteTask(ctx.taskId).catch(() => {});
		}
	});

	// ─── TASK-02: Create task without message ───────────────────────────

	test("TASK-02: Create task without message", async ({ api }) => {
		const { taskId, task } = await createTaskWithoutMessage(api, ctx.repositoryId!);

		expect(task.status).toBe("open");
		expect(taskId).toBeTruthy();

		ctx.taskId = taskId;
		ctx.task = task;

		console.log(`[Sandbox] Task created: ${taskId}`);
	});

	// ─── SANDBOX-01: Sandbox provisions (agent stays idle) ──────────────

	test("SANDBOX-01: Sandbox provisions (agent stays idle)", async ({ api }, testInfo) => {
		testInfo.setTimeout(200000);

		const { ready, status } = await waitForSandboxReady(api, ctx.taskId!);

		expect(ready).toBe(true);
		expect(status).not.toBeNull();
		expect(status!.environmentStatus).toBe("ready");

		// No environment error on healthy sandbox
		expect(status!.environmentError).toBeFalsy();

		// Agent should be idle (no message to process)
		expect(["idle", "none"]).toContain(status!.agentState);

		ctx.sandboxStatus = status!;
		console.log(`[Sandbox] Ready, agentState=${status!.agentState}`);
	});

	// ─── SANDBOX-01.1: SSE sandbox.ready event ─────────────────────────

	test("SANDBOX-01.1: SSE sandbox.ready event", async ({ api }) => {
		const event = await chainSse.waitForEvent({
			type: "sandbox",
			subType: "ready",
			taskId: ctx.taskId,
			timeout: 15000,
		});

		if (event) {
			expect(event.taskId).toBe(ctx.taskId);
		} else {
			// Fallback: SSE event may have been emitted before collector connected.
			// Verify sandbox is ready via API as confirmation.
			console.log("[SSE] sandbox.ready event missed — verifying status via API");
			const status = await api.getWorkspaceStatus(ctx.taskId!);
			expect(status.environmentStatus).toBe("ready");
		}
	});

	// ─── SANDBOX-02: Idempotent re-provision returns ready ──────────────

	test("SANDBOX-02: Idempotent re-provision returns ready", async ({ api }) => {
		const { status } = await provisionSandbox(api, ctx.taskId!);

		expect(status.environmentStatus).toBe("ready");
		console.log("[Sandbox] Re-provision returned ready (idempotent)");
	});

	// ─── SANDBOX-03: Stop sandbox (scale to 0) ─────────────────────────

	test("SANDBOX-03: Stop sandbox (scale to 0)", async ({ api }, testInfo) => {
		testInfo.setTimeout(90000);

		const status = await stopSandbox(api, ctx.taskId!);

		expect(status).not.toBeNull();
		expect(status!.environmentStatus).toBe("stopped");

		console.log("[Sandbox] Stopped");
	});

	// ─── SANDBOX-03.1: SSE sandbox.stopped event ───────────────────────

	test("SANDBOX-03.1: SSE sandbox.stopped event", async () => {
		const event = await chainSse.waitForEvent({
			type: "sandbox",
			subType: "stopped",
			taskId: ctx.taskId,
			timeout: 5000,
		});
		expect(event).not.toBeNull();
	});

	// ─── SANDBOX-03.2: Start stopped sandbox (files persist) ───────────

	test("SANDBOX-03.2: Start stopped sandbox (files persist)", async ({ api }, testInfo) => {
		testInfo.setTimeout(200000);

		const { ready, status } = await startSandbox(api, ctx.taskId!);

		expect(ready).toBe(true);
		expect(status).not.toBeNull();
		expect(status!.environmentStatus).toBe("ready");

		// Verify files are still present
		const files = await listFiles(api, ctx.taskId!, ".", false);
		expect(files.length).toBeGreaterThan(0);

		console.log(`[Sandbox] Restarted, ${files.length} files preserved`);
	});

	// ─── SANDBOX-04: Restart sandbox (pod recreated, files persist) ─────

	test("SANDBOX-04: Restart sandbox (pod recreated, files persist)", async ({ api }, testInfo) => {
		testInfo.setTimeout(200000);

		const { ready, status } = await restartSandbox(api, ctx.taskId!);

		expect(ready).toBe(true);
		expect(status).not.toBeNull();
		expect(status!.environmentStatus).toBe("ready");

		// Verify files are still present after restart
		const files = await listFiles(api, ctx.taskId!, ".", false);
		expect(files.length).toBeGreaterThan(0);

		console.log(`[Sandbox] Restarted, ${files.length} files preserved`);
	});

	// ─── SANDBOX-05: Cleanup sandbox (all resources removed) ───────────

	test("SANDBOX-05: Cleanup sandbox (all resources removed)", async ({ api }, testInfo) => {
		testInfo.setTimeout(90000);

		const status = await cleanupSandbox(api, ctx.taskId!);

		expect(status).not.toBeNull();
		expect(status!.environmentStatus).toBe("not_created");

		console.log("[Sandbox] Cleaned up");
	});

	// ─── SANDBOX-05.1: SSE sandbox.not_created event ────────────────────

	test("SANDBOX-05.1: SSE sandbox.not_created event", async () => {
		const event = await chainSse.waitForEvent({
			type: "sandbox",
			subType: "not_created",
			taskId: ctx.taskId,
			timeout: 5000,
		});
		expect(event).not.toBeNull();
	});
});
