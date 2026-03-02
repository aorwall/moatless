/**
 * Standalone: Task Error Cases (task-crud.md)
 *
 * Independent tests — each creates and tears down its own resources.
 *
 * Scenarios:
 *   TASK-10: Create task with invalid repository (404)
 *   TASK-11: Start closed task (400)
 */

import { test, expect } from "../fixtures/api-fixtures";
import {
	createTaskWithMessage,
	createTaskWithInvalidRepository,
	closeTask,
	sendMessageToClosedTask,
} from "../steps";

// ─── TASK-10: Create task with invalid repository (404) ─────────────────

test("TASK-10: Create task with invalid repository (404)", async ({ api }) => {
	const response = await createTaskWithInvalidRepository(
		api,
		"00000000-0000-0000-0000-000000000000",
	);

	expect(response.status).toBe(404);
	const body = await response.json();
	expect(body).toBeDefined();
});

// ─── TASK-11: Send message to closed task (accepted — backend reopens) ──

test("TASK-11: Send message to closed task (accepted)", async ({ api }) => {
	// Setup: create a task, then close it
	const repo = await api.ensureTemplateRepository();
	const { taskId } = await createTaskWithMessage(api, repo.id, "Temp task for close test");

	await closeTask(api, taskId);

	// Act: send message to the closed task — backend accepts and reopens
	const response = await sendMessageToClosedTask(api, taskId);

	expect(response.status).toBeLessThan(300);

	// Cleanup
	await api.deleteTask(taskId).catch(() => {});
});
