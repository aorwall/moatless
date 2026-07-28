/**
 * Task creation steps.
 *
 * A step exists where the call needs translating — here, a repository id goes
 * into the request as `templateId`, and the id a chain carries around comes back
 * nested. Everything else a chain does against `api` directly.
 */

import type { MoatApi } from '../../shared/moat-api'
import type { TaskResponse } from '../../shared/types'

// =============================================================================
// TASK-01: Create task with message
// =============================================================================

export async function createTaskWithMessage(
  api: MoatApi,
  repositoryId: string,
  message: string,
): Promise<{ taskId: string; task: TaskResponse }> {
  const task = await api.createVibeTask({
    templateId: repositoryId,
    message,
  })
  return { taskId: task.taskId, task }
}

// =============================================================================
// TASK-02: Create task without message
// =============================================================================

export async function createTaskWithoutMessage(
  api: MoatApi,
  repositoryId: string,
): Promise<{ taskId: string; task: TaskResponse }> {
  const task = await api.createVibeTask({
    templateId: repositoryId,
  })
  return { taskId: task.taskId, task }
}
