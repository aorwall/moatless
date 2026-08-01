/**
 * Chains 2-5: Webhook Inbound Routing
 *
 * Tests webhook inbound routing with by_subject and ongoing routing.
 *
 * The webhook route answers once the delivery is durably owned, which is before
 * any Task exists, so no assertion here reads a Task id out of a response.
 * Which Task a delivery reached is read from the Loop's recent-tasks view,
 * which returns each Task alongside the Binding — and therefore the subject —
 * that created it.
 */

import { ChainContext } from '../context/chain-context'
import { expect, test } from '../fixtures/api-fixtures'
import { discardTask } from '../helpers/teardown'
import {
  createWebhookTriggerFixtureWithRouting,
  sendWebhookMessage,
  taskMessageCount,
  waitForLoopTasks,
  waitForLoopTaskWithSubject,
  waitForTaskMessages,
  webhookSubjectKey,
} from '../steps'

async function waitForProcessing(ms = 1000): Promise<void> {
  await new Promise((resolve) => setTimeout(resolve, ms))
}

test.describe('Chain 2: Webhook by_subject routing without thread', () => {
  test.describe.configure({ mode: 'serial' })

  const ctx = new ChainContext()

  test.beforeAll(async ({ api }) => {
    const repo = await api.ensureTemplateRepository()
    ctx.repositoryId = repo.id
  })

  test.afterAll(async ({ api }) => {
    const triggerId = ctx.get<string>('triggerId')
    if (triggerId) {
      await api.deleteTaskTriggerFixture(triggerId).catch(() => {})
    }
    for (const taskId of ctx.get<string[]>('taskIds') ?? []) {
      await discardTask(api, taskId)
    }
  })

  test('INT-10: Create webhook config with by_subject routing', async ({ api }) => {
    const sourceId = `per-msg-${Date.now()}`
    const result = await createWebhookTriggerFixtureWithRouting(
      api,
      ctx.repositoryId!,
      'by_subject',
      { sourceId },
    )

    expect(result.config.active).toBe(true)
    expect(result.config.routingMode).toBe('by_subject')

    ctx.set('triggerId', result.triggerId)
    ctx.set('sourceId', sourceId)
    ctx.set('taskIds', [])
  })

  test('INT-11: Send message via webhook — task created', async ({ api }) => {
    const { response, deliveryId } = await sendWebhookMessage(
      api,
      ctx.get<string>('triggerId')!,
      'First by_subject webhook test',
    )

    expect(response.status).toBe(200)
    expect(deliveryId).toBeTruthy()

    const items = await waitForLoopTasks(api, ctx.get<string>('triggerId')!, 1)
    const taskId = items[0].task.taskId

    ctx.set('firstTaskId', taskId)
    ctx.set('taskIds', [taskId])
  })

  test('INT-11.1: Task is in open state', async ({ api }) => {
    const task = await api.getTask(ctx.get<string>('firstTaskId')!)

    expect(task.status).toBe('open')
  })

  test('INT-11.2: Task linked to correct repository', async ({ api }) => {
    const task = await api.getTask(ctx.get<string>('firstTaskId')!)

    expect(task.repositoryId).toBe(ctx.repositoryId)
  })

  test('INT-12: Send second message — new task created', async ({ api }) => {
    const { response } = await sendWebhookMessage(
      api,
      ctx.get<string>('triggerId')!,
      'Second by_subject webhook test',
    )

    expect(response.status).toBe(200)

    const items = await waitForLoopTasks(api, ctx.get<string>('triggerId')!, 2)
    const taskId = items
      .map((item) => item.task.taskId)
      .find((id) => id !== ctx.get<string>('firstTaskId'))
    expect(taskId).toBeTruthy()

    ctx.set('taskIds', [...(ctx.get<string[]>('taskIds') ?? []), taskId!])
  })

  test('INT-13: Send message with thread_id — no existing thread, new task', async ({ api }) => {
    const threadId = `new-thread-${Date.now()}`
    const { response } = await sendWebhookMessage(
      api,
      ctx.get<string>('triggerId')!,
      'Message with new thread',
      { threadId },
    )

    expect(response.status).toBe(200)

    const taskId = await waitForLoopTaskWithSubject(
      api,
      ctx.get<string>('triggerId')!,
      webhookSubjectKey(ctx.get<string>('sourceId')!, threadId),
    )
    expect(taskId).not.toBe(ctx.get<string>('firstTaskId'))

    ctx.set('taskIds', [...(ctx.get<string[]>('taskIds') ?? []), taskId])
  })
})

test.describe('Chain 3: Webhook ongoing routing', () => {
  test.describe.configure({ mode: 'serial' })

  const ctx = new ChainContext()

  test.beforeAll(async ({ api }) => {
    const repo = await api.ensureTemplateRepository()
    ctx.repositoryId = repo.id
  })

  test.afterAll(async ({ api }) => {
    const triggerId = ctx.get<string>('triggerId')
    if (triggerId) {
      await api.deleteTaskTriggerFixture(triggerId).catch(() => {})
    }
    for (const taskId of ctx.get<string[]>('taskIds') ?? []) {
      await discardTask(api, taskId)
    }
  })

  test('INT-20: Create webhook config with ongoing routing', async ({ api }) => {
    const result = await createWebhookTriggerFixtureWithRouting(api, ctx.repositoryId!, 'ongoing', {
      sourceId: `ongoing-${Date.now()}`,
    })

    expect(result.config.active).toBe(true)
    expect(result.config.routingMode).toBe('ongoing')

    ctx.set('triggerId', result.triggerId)
    ctx.set('taskIds', [])
  })

  test('INT-21: First message creates ongoing task', async ({ api }) => {
    const { response } = await sendWebhookMessage(
      api,
      ctx.get<string>('triggerId')!,
      'First ongoing message',
    )

    expect(response.status).toBe(200)

    const items = await waitForLoopTasks(api, ctx.get<string>('triggerId')!, 1)
    const taskId = items[0].task.taskId

    await waitForProcessing()

    const config = await api.getTaskTriggerFixture(ctx.get<string>('triggerId')!)
    expect(config.ongoingTaskId).toBe(taskId)

    ctx.set('ongoingTaskId', taskId)
    ctx.set('taskIds', [taskId])
  })

  test('INT-22: Second message routes to same task', async ({ api }) => {
    const ongoingTaskId = ctx.get<string>('ongoingTaskId')!
    const before = await taskMessageCount(api, ongoingTaskId)

    const { response } = await sendWebhookMessage(
      api,
      ctx.get<string>('triggerId')!,
      'Second ongoing message',
    )

    expect(response.status).toBe(200)

    await waitForTaskMessages(api, ongoingTaskId, before + 1)
    const items = await api.listLoopRecentTasks(ctx.get<string>('triggerId')!, 50)
    expect(items).toHaveLength(1)
    expect(items[0].task.taskId).toBe(ongoingTaskId)

    const config = await api.getTaskTriggerFixture(ctx.get<string>('triggerId')!)
    expect(config.ongoingTaskId).toBe(ongoingTaskId)
  })

  test('INT-23: After task closed, next message creates new task', async ({ api }) => {
    const ongoingTaskId = ctx.get<string>('ongoingTaskId')!
    await api.closeTask(ongoingTaskId)

    const { response } = await sendWebhookMessage(
      api,
      ctx.get<string>('triggerId')!,
      'Message after close',
    )

    expect(response.status).toBe(200)

    const items = await waitForLoopTasks(api, ctx.get<string>('triggerId')!, 2)
    const taskId = items.map((item) => item.task.taskId).find((id) => id !== ongoingTaskId)
    expect(taskId).toBeTruthy()

    await waitForProcessing()

    const config = await api.getTaskTriggerFixture(ctx.get<string>('triggerId')!)
    expect(config.ongoingTaskId).toBe(taskId)

    ctx.set('taskIds', [...(ctx.get<string[]>('taskIds') ?? []), taskId!])
  })
})

test.describe('Chain 4: Webhook by_subject routing with thread', () => {
  test.describe.configure({ mode: 'serial' })

  const ctx = new ChainContext()

  test.beforeAll(async ({ api }) => {
    const repo = await api.ensureTemplateRepository()
    ctx.repositoryId = repo.id
  })

  test.afterAll(async ({ api }) => {
    const triggerId = ctx.get<string>('triggerId')
    if (triggerId) {
      await api.deleteTaskTriggerFixture(triggerId).catch(() => {})
    }
    for (const taskId of ctx.get<string[]>('taskIds') ?? []) {
      await discardTask(api, taskId)
    }
  })

  test('INT-30: Create webhook config with by_subject routing', async ({ api }) => {
    const sourceId = `thread-${Date.now()}`
    const result = await createWebhookTriggerFixtureWithRouting(
      api,
      ctx.repositoryId!,
      'by_subject',
      { sourceId },
    )

    expect(result.config.active).toBe(true)
    expect(result.config.routingMode).toBe('by_subject')

    ctx.set('triggerId', result.triggerId)
    ctx.set('sourceId', sourceId)
    ctx.set('taskIds', [])
  })

  test('INT-31: New message creates task', async ({ api }) => {
    const threadId = `thread-a-${Date.now()}`

    const { response } = await sendWebhookMessage(
      api,
      ctx.get<string>('triggerId')!,
      'First thread message',
      { threadId },
    )

    expect(response.status).toBe(200)

    const taskId = await waitForLoopTaskWithSubject(
      api,
      ctx.get<string>('triggerId')!,
      webhookSubjectKey(ctx.get<string>('sourceId')!, threadId),
    )

    ctx.set('threadId', threadId)
    ctx.set('firstTaskId', taskId)
    ctx.set('taskIds', [taskId])
  })

  test('INT-32: Reply in same thread routes to same task', async ({ api }) => {
    const firstTaskId = ctx.get<string>('firstTaskId')!
    const before = await taskMessageCount(api, firstTaskId)

    const { response } = await sendWebhookMessage(
      api,
      ctx.get<string>('triggerId')!,
      'Reply in same thread',
      { threadId: ctx.get<string>('threadId') },
    )

    expect(response.status).toBe(200)

    await waitForTaskMessages(api, firstTaskId, before + 1)
    const items = await api.listLoopRecentTasks(ctx.get<string>('triggerId')!, 50)
    expect(items).toHaveLength(1)
    expect(items[0].task.taskId).toBe(firstTaskId)
  })

  test('INT-33: New thread creates new task', async ({ api }) => {
    const newThreadId = `thread-b-${Date.now()}`

    const { response } = await sendWebhookMessage(
      api,
      ctx.get<string>('triggerId')!,
      'Message in new thread',
      { threadId: newThreadId },
    )

    expect(response.status).toBe(200)

    const taskId = await waitForLoopTaskWithSubject(
      api,
      ctx.get<string>('triggerId')!,
      webhookSubjectKey(ctx.get<string>('sourceId')!, newThreadId),
    )
    expect(taskId).not.toBe(ctx.get<string>('firstTaskId'))

    ctx.set('taskIds', [...(ctx.get<string[]>('taskIds') ?? []), taskId])
  })
})

test.describe('Chain 5: Webhook delivery admission', () => {
  test.describe.configure({ mode: 'serial' })

  const ctx = new ChainContext()

  test.beforeAll(async ({ api }) => {
    const repo = await api.ensureTemplateRepository()
    ctx.repositoryId = repo.id
  })

  test.afterAll(async ({ api }) => {
    const triggerId = ctx.get<string>('triggerId')
    if (triggerId) {
      await api.deleteTaskTriggerFixture(triggerId).catch(() => {})
    }
    for (const taskId of ctx.get<string[]>('taskIds') ?? []) {
      await discardTask(api, taskId)
    }
  })

  test('INT-40: Create webhook config for admission tests', async ({ api }) => {
    const sourceId = `admission-${Date.now()}`
    const result = await createWebhookTriggerFixtureWithRouting(
      api,
      ctx.repositoryId!,
      'by_subject',
      { sourceId },
    )

    ctx.set('triggerId', result.triggerId)
    ctx.set('sourceId', sourceId)
    ctx.set('taskIds', [])
  })

  test('INT-41: A supplied delivery id makes a retry a duplicate', async ({ api }) => {
    const triggerId = ctx.get<string>('triggerId')!
    const threadId = `dedup-${Date.now()}`
    const deliveryId = `delivery-${Date.now()}`

    const first = await sendWebhookMessage(api, triggerId, 'deliver once', {
      threadId,
      deliveryId,
    })
    expect(first.response.status).toBe(200)
    expect(first.deliveryId).toBe(deliveryId)
    expect(first.admission).toBe('admitted')

    const taskId = await waitForLoopTaskWithSubject(
      api,
      triggerId,
      webhookSubjectKey(ctx.get<string>('sourceId')!, threadId),
    )
    ctx.set('taskIds', [...(ctx.get<string[]>('taskIds') ?? []), taskId])
    const afterFirst = await waitForTaskMessages(api, taskId, 1)

    const retry = await sendWebhookMessage(api, triggerId, 'deliver once', {
      threadId,
      deliveryId,
    })
    expect(retry.response.status).toBe(200)
    expect(retry.admission).toBe('duplicate')
    expect(await taskMessageCount(api, taskId)).toBe(afterFirst)
  })

  test('INT-42: Two deliveries without an identifier are two deliveries', async ({ api }) => {
    const triggerId = ctx.get<string>('triggerId')!
    const threadId = `no-id-${Date.now()}`

    const first = await sendWebhookMessage(api, triggerId, 'deliver twice', { threadId })
    expect(first.response.status).toBe(200)

    const taskId = await waitForLoopTaskWithSubject(
      api,
      triggerId,
      webhookSubjectKey(ctx.get<string>('sourceId')!, threadId),
    )
    ctx.set('taskIds', [...(ctx.get<string[]>('taskIds') ?? []), taskId])
    const afterFirst = await waitForTaskMessages(api, taskId, 1)

    const second = await sendWebhookMessage(api, triggerId, 'deliver twice', { threadId })
    expect(second.response.status).toBe(200)
    expect(second.admission).toBe('admitted')
    expect(second.deliveryId).not.toBe(first.deliveryId)
    await waitForTaskMessages(api, taskId, afterFirst + 1)
  })
})

test('INT-34: A reply after close starts a new task', async ({ api }) => {
  const repo = await api.ensureTemplateRepository()
  const suffix = Date.now()
  const sourceId = `closed-${suffix}`
  const { triggerId } = await createWebhookTriggerFixtureWithRouting(api, repo.id, 'by_subject', {
    sourceId,
  })
  const thread = `thread-closed-${suffix}`
  const created: string[] = []

  try {
    const first = await sendWebhookMessage(api, triggerId, 'First message', {
      threadId: thread,
    })
    expect(first.response.status).toBe(200)
    const firstTaskId = await waitForLoopTaskWithSubject(
      api,
      triggerId,
      webhookSubjectKey(sourceId, thread),
    )
    created.push(firstTaskId)

    const before = (await api.getTaskMessages(firstTaskId)).messages.length

    await api.closeTask(firstTaskId)
    expect((await api.getTask(firstTaskId)).status).toBe('closed')

    const reply = await sendWebhookMessage(api, triggerId, 'Reply after close', {
      threadId: thread,
    })
    expect(reply.response.status).toBe(200)

    const items = await waitForLoopTasks(api, triggerId, 2)
    const replyTaskId = items.map((item) => item.task.taskId).find((id) => id !== firstTaskId)
    expect(replyTaskId, 'a reply after close must not resurrect the closed task').toBeTruthy()
    created.push(replyTaskId!)

    const still = await api.getTask(firstTaskId)
    expect(still.status).toBe('closed')
    expect((await api.getTaskMessages(firstTaskId)).messages).toHaveLength(before)
  } finally {
    for (const taskId of created) {
      await discardTask(api, taskId)
    }
    await api.deleteTaskTriggerFixture(triggerId).catch(() => {})
  }
})
