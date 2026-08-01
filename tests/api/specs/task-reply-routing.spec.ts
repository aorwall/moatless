/**
 * Standalone: Task reply routing
 *
 * A reply has to go somewhere. Where is decided by the task's bindings: a task
 * with none has no destination, a task with one needs no choosing, and a task
 * with several has to be told which — the error that says so carries the
 * candidates and the flag that picks between them, because it is written for
 * somebody at a terminal.
 *
 * Scenarios:
 *   INT-40: Replying to a task with no binding answers 404
 *   INT-41: Replying to a task with one binding sends to it
 *   INT-42: Replying to a task with several bindings answers 400 and lists them
 *   INT-43: Naming the binding explicitly sends to it
 *   INT-44: Naming a binding that does not exist answers 404
 */

import type { ReplyTargetDescriptor } from '@moatless/api-client/sandbox/model'
import type { MoatApi } from '../../shared/moat-api'
import { expect, test } from '../fixtures/api-fixtures'
import { discardTask } from '../helpers/teardown'
import { createWebhookTriggerFixture } from '../steps'

/** A task with a message and no binding of any kind. */
async function bareTask(api: MoatApi): Promise<string> {
  const repo = await api.ensureTemplateRepository()
  const task = await api.createVibeTask({
    templateId: repo.id,
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

// ─── INT-41: Replying to a task with one binding sends to it ────────────

test('INT-41: Replying to a task with one binding sends to it', async ({ api }) => {
  const repo = await api.ensureTemplateRepository()
  const suffix = Date.now()
  const { triggerId } = await createWebhookTriggerFixture(api, repo.id, {
    sourceId: `reply-single-${suffix}`,
  })

  // The inbound message is what creates both the task and its one binding —
  // which is the case worth testing, since it is how a real reply target
  // comes to exist.
  const inbound = await api.sendWebhookMessage(triggerId, {
    text: 'seed one binding',
    threadId: `thread-${suffix}`,
  })
  expect(inbound.status).toBe(200)
  const taskId = (inbound.body as { taskId: string }).taskId

  try {
    const bindings = await api.listTaskBindings(taskId)
    expect(bindings).toHaveLength(1)

    const result = await api.sendTaskReply(taskId, {
      text: 'single-binding reply',
      fileIds: [],
    })

    expect(result.sentCount).toBe(1)
    expect(result.errors).toEqual([])
    expect(result.targets).toHaveLength(1)
    expect(result.targets?.[0]?.kind).toBe('binding')
  } finally {
    await discard(api, taskId)
    await api.deleteLoop(triggerId).catch(() => {})
  }
})

// ─── INT-42: Several bindings answer 400 and list them ──────────────────

test('INT-42: Replying to a task with several bindings answers 400 and lists them', async ({
  api,
}) => {
  const repo = await api.ensureTemplateRepository()
  const suffix = Date.now()
  const taskId = await bareTask(api)
  const { triggerId, config } = await createWebhookTriggerFixture(api, repo.id, {
    sourceId: `reply-multi-${suffix}`,
  })

  try {
    for (const index of [1, 2]) {
      await api.createTaskBinding(taskId, {
        adapterKind: 'webhook',
        connectionId: config.connectionId,
        triggerId,
        subjectKind: 'webhook_thread',
        subjectKey: `manual-${suffix}-${index}`,
        destinationConfig: {},
        origin: 'test',
      })
    }

    const response = await api.sendTaskReplyRaw(taskId, {
      text: 'should fail - multiple bindings',
      fileIds: [],
    })

    expect(response.status).toBe(400)
    const body = response.body as { targets: ReplyTargetDescriptor[] }
    expect(body.targets).toHaveLength(2)
    for (const target of body.targets) {
      expect(target.kind).toBe('binding')
      // The hint is a command fragment, not explanatory text — it is meant to be pasted.
      expect(target.flagHint.startsWith('--binding ')).toBe(true)
    }
  } finally {
    await discard(api, taskId)
    await api.deleteLoop(triggerId).catch(() => {})
  }
})

// ─── INT-43: Naming the binding explicitly sends to it ──────────────────

test('INT-43: Naming the binding explicitly sends to it', async ({ api }) => {
  const repo = await api.ensureTemplateRepository()
  const suffix = Date.now()
  const taskId = await bareTask(api)
  const { triggerId, config } = await createWebhookTriggerFixture(api, repo.id, {
    sourceId: `reply-explicit-${suffix}`,
  })

  try {
    const binding = await api.createTaskBinding(taskId, {
      adapterKind: 'webhook',
      connectionId: config.connectionId,
      triggerId,
      subjectKind: 'webhook_thread',
      subjectKey: `manual-${suffix}`,
      destinationConfig: {},
      origin: 'test',
    })

    const result = await api.sendTaskReply(taskId, {
      text: 'explicit-binding reply',
      bindingId: binding.id,
      fileIds: [],
    })

    expect(result.sentCount).toBe(1)
  } finally {
    await discard(api, taskId)
    await api.deleteLoop(triggerId).catch(() => {})
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
