/**
 * Standalone: Task reply routing
 *
 * A reply has to go somewhere. Where is decided by the task's bindings: a task
 * with none has no destination, and a binding that does not exist is not one.
 *
 * Scenarios:
 *   INT-40: Replying to a task with no binding answers 404
 *   INT-44: Naming a binding that does not exist answers 404
 */

import type { MoatApi } from '../../shared/moat-api'
import { expect, test } from '../fixtures/api-fixtures'
import { discardTask } from '../helpers/teardown'

/** A task with a message and no binding of any kind. */
async function bareTask(api: MoatApi): Promise<string> {
  const workspace = await api.ensureTemplateWorkspace()
  const task = await api.createVibeTask({
    workspaceId: workspace.id,
    name: 'reply-routing-test',
    message: 'Reply-routing fixture task',
  })
  return task.taskId
}

async function discard(api: MoatApi, taskId: string): Promise<void> {
  await discardTask(api, taskId)
}

// ─── INT-40: Replying to a task with no binding answers 404 ─────────────

test('INT-40: Replying to a task with no binding answers 404', async ({ api }) => {
  const taskId = await bareTask(api)

  try {
    const response = await api.sendTaskReplyRaw(taskId, {
      text: 'should fail - no bindings',
      fileIds: [],
    })

    expect(response.status).toBe(404)
    const body = response.body as { error?: string }
    expect(body.error ?? '').toContain('no bindings')
  } finally {
    await discard(api, taskId)
  }
})

// ─── INT-44: Naming a binding that does not exist answers 404 ───────────

test('INT-44: Naming a binding that does not exist answers 404', async ({ api }) => {
  const taskId = await bareTask(api)

  try {
    const response = await api.sendTaskReplyRaw(taskId, {
      text: 'binding that does not exist',
      bindingId: 'tb_does_not_exist',
      fileIds: [],
    })

    expect(response.status).toBe(404)
  } finally {
    await discard(api, taskId)
  }
})
