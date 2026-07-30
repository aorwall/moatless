import type { TaskResponse } from '@moatless/api-client'

/**
 * Label for the workspace a task runs in, for task-facing surfaces (sidebar
 * rows, task page header).
 *
 * The backend names a workspace on every task — a task created from a bare
 * repository gets that repository's shadow workspace. `workspaceName` is still
 * absent when the workspace row was deleted or its lookup failed, so fall back
 * to the repository rather than rendering a blank label.
 */
export function taskWorkspaceLabel(
  task: Pick<TaskResponse, 'workspaceName' | 'repositoryName' | 'repositoryId'>,
): string {
  return task.workspaceName || task.repositoryName || task.repositoryId
}
