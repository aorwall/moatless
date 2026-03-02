/**
 * Chain: Task CRUD (task-crud.md)
 *
 * Pure API chain — no sandbox or agent execution needed.
 * Tests task creation, retrieval, updates, tagging, visibility, and deletion.
 *
 * Chain:
 *   TASK-01 → TASK-01.1 → TASK-01.2 → TASK-03 → TASK-04 →
 *   TASK-05 → TASK-05.1 → TASK-06 → TASK-07 → TASK-08 →
 *   TASK-08.1 → TASK-09 → TASK-09.1
 */

import { test, expect } from "../fixtures/api-fixtures";
import { getBaseURL } from "../fixtures/api-fixtures";
import { ChainContext } from "../context/chain-context";
import { SSEEventCollector } from "../../shared/sse-collector";
import {
	createTaskWithMessage,
	getTask,
	listTasks,
	updateTaskMetadata,
	setTaskTags,
	clearTaskTags,
	updateVisibility,
	closeTask,
	deleteTask,
} from "../steps";

test.describe("Chain: Task CRUD", () => {
	test.describe.configure({ mode: "serial" });

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
			await api.deleteTask(ctx.taskId).catch(() => {});
		}
	});

	// ─── TASK-01: Create task with message ──────────────────────────────

	test("TASK-01: Create task with message", async ({ api }) => {
		const result = await createTaskWithMessage(
			api,
			ctx.repositoryId!,
			"E2E CRUD test task",
		);

		expect(result.task.status).toBe("open");
		expect(result.taskId).toBeTruthy();

		ctx.taskId = result.taskId;
		ctx.task = result.task;
	});

	// ─── TASK-01.1: Task appears in listing ─────────────────────────────

	test("TASK-01.1: Task appears in listing", async ({ api }) => {
		const result = await listTasks(api);

		const found = result.items.some((t) => t.taskId === ctx.taskId);
		expect(found).toBe(true);
	});

	// ─── TASK-01.2: SSE task.created event emitted ──────────────────────

	test("TASK-01.2: SSE task.created event emitted", async () => {
		const event = await chainSse.waitForEvent({
			type: "task",
			taskId: ctx.taskId,
			timeout: 5000,
		});

		expect(event).not.toBeNull();
		expect(event!.taskId).toBe(ctx.taskId);
	});

	// ─── TASK-03: Get task by ID ────────────────────────────────────────

	test("TASK-03: Get task by ID", async ({ api }) => {
		const task = await getTask(api, ctx.taskId!);

		expect(task.taskId).toBe(ctx.taskId);
		expect(task.status).toBe("open");
		expect(task.repositoryId).toBeTruthy();
		expect(task.createdAt).toBeTruthy();
	});

	// ─── TASK-04: Update task metadata ──────────────────────────────────

	test("TASK-04: Update task metadata", async ({ api }) => {
		const updated = await updateTaskMetadata(api, ctx.taskId!, {
			name: "Updated CRUD Test Task",
		});

		expect(updated.name).toBe("Updated CRUD Test Task");

		// Confirm via GET
		const fetched = await getTask(api, ctx.taskId!);
		expect(fetched.name).toBe("Updated CRUD Test Task");
	});

	// ─── TASK-05: Set task tags ─────────────────────────────────────────

	test("TASK-05: Set task tags", async ({ api }) => {
		await setTaskTags(api, ctx.taskId!, ["e2e", "crud-test"]);

		const task = await getTask(api, ctx.taskId!);
		expect(task.tags).toBeDefined();
		expect(task.tags!.length).toBeGreaterThanOrEqual(2);

		const tagNames = task.tags!.map((t) => t.name);
		expect(tagNames).toContain("e2e");
		expect(tagNames).toContain("crud-test");
	});

	// ─── TASK-05.1: Clear tags ──────────────────────────────────────────

	test("TASK-05.1: Clear tags", async ({ api }) => {
		await clearTaskTags(api, ctx.taskId!);

		const task = await getTask(api, ctx.taskId!);
		expect(task.tags ?? []).toHaveLength(0);
	});

	// ─── TASK-06: Update task visibility ────────────────────────────────

	test("TASK-06: Update task visibility", async ({ api }) => {
		const updated = await updateVisibility(api, ctx.taskId!, "public");

		expect(updated.visibility).toBe("public");

		// Confirm via GET
		const fetched = await getTask(api, ctx.taskId!);
		expect(fetched.visibility).toBe("public");
	});

	// ─── TASK-07: List tasks with filters ───────────────────────────────

	test("TASK-07: List tasks with filters", async ({ api }) => {
		const result = await listTasks(api, { status: "open" });

		expect(result.items.length).toBeGreaterThan(0);
		for (const task of result.items) {
			expect(task.status).toBe("open");
		}
	});

	// ─── TASK-08: Close task ────────────────────────────────────────────

	test("TASK-08: Close task", async ({ api }) => {
		const closed = await closeTask(api, ctx.taskId!);

		expect(closed.status).toBe("closed");

		// Confirm via GET
		const fetched = await getTask(api, ctx.taskId!);
		expect(fetched.status).toBe("closed");
	});

	// ─── TASK-08.1: SSE task.closed event ───────────────────────────────

	test("TASK-08.1: SSE task.closed event", async () => {
		const event = await chainSse.waitForEvent({
			type: "task",
			subType: "closed",
			taskId: ctx.taskId,
			timeout: 5000,
		});

		const taskEvents = chainSse.getEventsForTask(ctx.taskId!);
		const hasClosedEvent = taskEvents.some(
			(e) => e.type === "task" && (e.subType === "closed" || e.subType === "updated"),
		);
		expect(event !== null || hasClosedEvent).toBe(true);
	});

	// ─── TASK-09: Delete task ───────────────────────────────────────────

	test("TASK-09: Delete task", async ({ api }) => {
		const response = await deleteTask(api, ctx.taskId!);

		expect(response.status).toBeLessThan(300);
	});

	// ─── TASK-09.1: Task removed from listing ───────────────────────────

	test("TASK-09.1: Task removed from listing", async ({ api }) => {
		const result = await listTasks(api);

		const found = result.items.some((t) => t.taskId === ctx.taskId);
		expect(found).toBe(false);

		// Prevent afterAll from trying to delete again
		ctx.taskId = undefined;
	});
});
