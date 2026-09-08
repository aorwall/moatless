/**
 * Integration step functions (INT-01 through INT-54).
 *
 * Each function performs an action and returns a result.
 * Tests add assertions; preconditions skip them.
 */

import type { LoopRecentTaskItem } from '@moatless/api-client/sandbox/model'
import type { RawResponse } from '../../shared/moat'
import type { MoatApi } from '../../shared/moat-api'
import type { TaskLoopFixtureResponse } from '../../shared/types'

// =============================================================================
// INT-01: Create webhook subscription loop fixture
// =============================================================================

export async function createWebhookTriggerFixture(
  api: MoatApi,
  workspaceId: string,
  options?: {
    sourceId?: string
    sourceName?: string
    routingMode?: string
    promptTemplate?: string
    adapterConfig?: Record<string, unknown>
  },
): Promise<{ triggerId: string; loopId: string; config: TaskLoopFixtureResponse }> {
  const config = await api.createTaskTriggerFixture({
    adapterKind: 'webhook',
    sourceId: options?.sourceId ?? `webhook-${Date.now()}`,
    sourceName: options?.sourceName,
    workspaceId,
    routingMode: options?.routingMode ?? 'by_subject',
    promptTemplate: options?.promptTemplate,
    adapterConfig: options?.adapterConfig,
  })
  return { triggerId: config.id, loopId: config.id, config }
}

// =============================================================================
// INT-10/20/30: Create fixture with specific routing mode
// =============================================================================

export async function createWebhookTriggerFixtureWithRouting(
  api: MoatApi,
  workspaceId: string,
  routingMode: string,
  options?: {
    sourceId?: string
    adapterConfig?: Record<string, unknown>
  },
): Promise<{ triggerId: string; loopId: string; config: TaskLoopFixtureResponse }> {
  return createWebhookTriggerFixture(api, workspaceId, {
    routingMode,
    sourceId: options?.sourceId,
    adapterConfig: options?.adapterConfig,
  })
}

// =============================================================================
// INT-11: Send message via webhook
// =============================================================================

/**
 * Send one webhook delivery.
 *
 * The route answers once the delivery is durably owned — before anything
 * routes or creates a Task — so the response names the delivery, not a Task.
 * Which Task the delivery reached is read afterwards, from the Loop's
 * recent-tasks view.
 */
export async function sendWebhookMessage(
  api: MoatApi,
  triggerId: string,
  text: string,
  options?: {
    threadId?: string
    userId?: string
    metadata?: Record<string, unknown>
    deliveryId?: string
  },
): Promise<{
  response: RawResponse
  deliveryId: string | null
  admission: string | null
}> {
  const response = await api.sendWebhookMessage(triggerId, {
    text,
    threadId: options?.threadId,
    userId: options?.userId,
    metadata: options?.metadata,
    deliveryId: options?.deliveryId,
  })

  let deliveryId: string | null = null
  let admission: string | null = null
  if (response.status < 400 && response.body && typeof response.body === 'object') {
    const body = response.body as { deliveryId?: string; status?: string }
    deliveryId = body.deliveryId ?? null
    admission = body.status ?? null
  }

  return { response, deliveryId, admission }
}

/** The subject a webhook delivery routes on, as the inbound route composes it. */
export function webhookSubjectKey(sourceId: string, threadId: string): string {
  return `${sourceId}:${threadId}`
}

const ROUTING_TIMEOUT_MS = 60_000
const POLL_INTERVAL_MS = 200

async function poll<T>(describe: string, attempt: () => Promise<T | null>): Promise<T> {
  const deadline = Date.now() + ROUTING_TIMEOUT_MS
  for (;;) {
    const result = await attempt()
    if (result !== null) return result
    if (Date.now() >= deadline) {
      throw new Error(`${describe} within ${ROUTING_TIMEOUT_MS}ms`)
    }
    await new Promise((resolve) => setTimeout(resolve, POLL_INTERVAL_MS))
  }
}

/** Wait until the Loop's recent-tasks view shows at least `expected` Tasks. */
export async function waitForLoopTasks(
  api: MoatApi,
  triggerId: string,
  expected: number,
): Promise<LoopRecentTaskItem[]> {
  return poll(`loop ${triggerId} did not produce ${expected} tasks`, async () => {
    const items = await api.listLoopRecentTasks(triggerId, 50)
    return items.length >= expected ? items : null
  })
}

/** Wait until a Task bound to `subjectKey` appears, and return its id. */
export async function waitForLoopTaskWithSubject(
  api: MoatApi,
  triggerId: string,
  subjectKey: string,
): Promise<string> {
  return poll(`no task bound to \`${subjectKey}\` on loop ${triggerId}`, async () => {
    const items = await api.listLoopRecentTasks(triggerId, 50)
    const found = items.find((item) => item.binding?.subjectKey === subjectKey)
    return found?.task.taskId ?? null
  })
}

/**
 * Wait until a Task holds at least `expected` messages.
 *
 * A second delivery to an existing subject adds a message rather than a Task,
 * so this is how a test observes that it landed.
 */
export async function waitForTaskMessages(
  api: MoatApi,
  taskId: string,
  expected: number,
): Promise<number> {
  return poll(`task ${taskId} did not reach ${expected} messages`, async () => {
    const { messages } = await api.getTaskMessages(taskId)
    return messages.length >= expected ? messages.length : null
  })
}

/** How many messages a Task currently holds. */
export async function taskMessageCount(api: MoatApi, taskId: string): Promise<number> {
  const { messages } = await api.getTaskMessages(taskId)
  return messages.length
}
