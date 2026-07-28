/**
 * Teardown a chain runs after it has already reported its result.
 *
 * Nothing here may throw: a chain that passed must not be turned red by a
 * cleanup call, and a chain that failed has already said why. Every call
 * swallows its own error, and what is left behind instead gets collected by
 * `tests/sweep.ts`.
 */

import type { MoatApi } from '../../shared/moat-api'

/**
 * Stop a task and delete it, whatever state it is in.
 *
 * The pause between the two is the sandbox's: a delete that lands while the
 * stop is still propagating can leave the sandbox running.
 */
export async function discardTask(api: MoatApi, taskId: string | undefined): Promise<void> {
  if (!taskId) {
    return
  }
  await api.stopTask(taskId).catch(() => {})
  await new Promise((resolve) => setTimeout(resolve, 500))
  await api.deleteTask(taskId).catch(() => {})
}
