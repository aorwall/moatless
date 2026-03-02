/**
 * Task step functions (TASK-01 through TASK-11).
 *
 * Each function performs an action and returns a result.
 * Tests add assertions; preconditions skip them.
 */

import type {
	ApiClient,
	ListTasksResponse,
	TaskResponse,
} from "../../shared/api-client";

// =============================================================================
// TASK-01: Create task with message
// =============================================================================

export async function createTaskWithMessage(
	api: ApiClient,
	repositoryId: string,
	message: string,
): Promise<{ taskId: string; task: TaskResponse }> {
	const task = await api.createVibeTask({
		templateId: repositoryId,
		message,
	});
	return { taskId: task.taskId, task };
}

// =============================================================================
// TASK-02: Create task without message
// =============================================================================

export async function createTaskWithoutMessage(
	api: ApiClient,
	repositoryId: string,
): Promise<{ taskId: string; task: TaskResponse }> {
	const task = await api.createVibeTask({
		templateId: repositoryId,
	});
	return { taskId: task.taskId, task };
}

// =============================================================================
// TASK-03: Get task by ID
// =============================================================================

export async function getTask(
	api: ApiClient,
	taskId: string,
): Promise<TaskResponse> {
	return api.getTask(taskId);
}

// =============================================================================
// TASK-04: Update task metadata
// =============================================================================

export async function updateTaskMetadata(
	api: ApiClient,
	taskId: string,
	data: { name?: string; description?: string },
): Promise<TaskResponse> {
	return api.updateTaskMetadata(taskId, data);
}

// =============================================================================
// TASK-05: Set task tags
// =============================================================================

export async function setTaskTags(
	api: ApiClient,
	taskId: string,
	tags: string[],
): Promise<unknown> {
	return api.setTaskTags(taskId, tags);
}

// =============================================================================
// TASK-05.1: Clear tags
// =============================================================================

export async function clearTaskTags(
	api: ApiClient,
	taskId: string,
): Promise<unknown> {
	return api.setTaskTags(taskId, []);
}

// =============================================================================
// TASK-06: Update task visibility
// =============================================================================

export async function updateVisibility(
	api: ApiClient,
	taskId: string,
	visibility: "private" | "public",
): Promise<TaskResponse> {
	return api.updateVisibility(taskId, visibility);
}

// =============================================================================
// TASK-07: List tasks with filters
// =============================================================================

export async function listTasks(
	api: ApiClient,
	filters?: { status?: string; repositoryId?: string; tag?: string },
): Promise<ListTasksResponse> {
	return api.listTasks(filters);
}

// =============================================================================
// TASK-08: Close task
// =============================================================================

export async function closeTask(
	api: ApiClient,
	taskId: string,
): Promise<TaskResponse> {
	return api.closeTask(taskId);
}

// =============================================================================
// TASK-09: Delete task
// =============================================================================

export async function deleteTask(
	api: ApiClient,
	taskId: string,
): Promise<Response> {
	return api.deleteTaskRaw(taskId);
}

// =============================================================================
// TASK-10: Create task with invalid repository
// =============================================================================

export async function createTaskWithInvalidRepository(
	api: ApiClient,
	fakeRepositoryId: string,
): Promise<Response> {
	return api.createVibeTaskRaw({
		repositoryId: fakeRepositoryId,
		message: "This should fail",
	});
}

// =============================================================================
// TASK-11: Send message to closed task
// =============================================================================

export async function sendMessageToClosedTask(
	api: ApiClient,
	taskId: string,
): Promise<Response> {
	return api.sendMessageRaw(taskId, "This should fail because task is closed");
}
