import type { Task } from '@moatless/types'

/**
 * Derive task status based on the last iteration's status.
 *
 * Status Derivation Rules:
 * - Closed tasks remain closed regardless of iteration status
 * - If last iteration is "active", task status is "running"
 * - If task status is "running" but last iteration is not active:
 *   - Set to "error" if last iteration failed
 *   - Set to "open" otherwise
 * - For other statuses, keep the existing task status
 *
 * @param task - Task to derive status for
 * @returns Task with derived status
 */
export function deriveTaskStatus(task: Task): Task {
  // Closed tasks should remain closed regardless of iteration status
  if (task.status === 'closed') {
    return task
  }

  if (!task.iterations || task.iterations.length === 0) {
    return task
  }

  // Get the last iteration
  const lastIteration = task.iterations[task.iterations.length - 1]

  // If the last iteration is active, the task is running
  if (lastIteration && lastIteration.status === 'active') {
    return {
      ...task,
      status: 'running',
    }
  }

  // If task status is 'running' but last iteration is not active,
  // derive status back to 'open' (or 'error' if iteration failed)
  // This handles cases where the task was saved with 'running' status
  // but the iteration has since completed/failed
  if (task.status === 'running') {
    const newStatus = lastIteration?.status === 'failed' ? 'error' : 'open'
    return {
      ...task,
      status: newStatus,
    }
  }

  // For other statuses, keep the existing task status
  return task
}
