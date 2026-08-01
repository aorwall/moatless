/**
 * Chain 6: Webhook Outbound Callback (integrations-triggers-bindings.md)
 *
 * Tests that the backend delivers outbound callback POSTs to the
 * webhook config's callback_url when an agent completes a task.
 *
 * The callback server is hosted by the test process, so the backend has to be
 * able to reach back into it, and a real agent turn has to run to trigger the
 * delivery. Skipped where either is missing.
 *
 * Chain:
 *   INT-60 → INT-61 → INT-62 → INT-63
 */

import { missingCapability, profile } from '../../shared/profile'
import { waitForAgentComplete } from '../../shared/wait-helpers'
import { WebhookCallbackServer } from '../../shared/webhook-callback-server'
import { ChainContext } from '../context/chain-context'
import { expect, test } from '../fixtures/api-fixtures'
import { progress } from '../helpers/progress'
import { discardTask } from '../helpers/teardown'
import {
  createWebhookTriggerFixture,
  sendWebhookMessage,
  waitForLoopTasks,
  waitForSandboxReady,
} from '../steps'

const callbackReason = missingCapability('inboundCallbacks', 'agentCredentials')

test.describe('Chain 6: Webhook Outbound Callback', () => {
  test.describe.configure({ mode: 'serial' })

  test.skip(callbackReason !== null, callbackReason ?? '')

  const ctx = new ChainContext()
  let callbackServer: WebhookCallbackServer

  test.beforeAll(async ({ api }) => {
    // Start the callback server
    callbackServer = new WebhookCallbackServer({
      debug: !!process.env.DEBUG_CALLBACK,
    })
    await callbackServer.start()

    // Set up repository
    const repo = await api.ensureTemplateRepository()
    ctx.repositoryId = repo.id
  })

  test.afterAll(async ({ api }) => {
    // Clean up callback server
    await callbackServer?.stop()

    // Clean up subscription loop fixture
    const triggerId = ctx.get<string>('triggerId')
    if (triggerId) {
      await api.deleteTaskTriggerFixture(triggerId).catch(() => {})
    }

    // Clean up tasks
    for (const taskId of ctx.get<string[]>('taskIds') ?? []) {
      await discardTask(api, taskId)
    }
  })

  // ─── INT-60: Create webhook config with callback URL ────────────────

  test('INT-60: Create webhook config with callback URL', async ({ api }) => {
    const result = await createWebhookTriggerFixture(api, ctx.repositoryId!, {
      sourceId: `callback-test-${Date.now()}`,
      adapterConfig: {
        callback_url: callbackServer.url,
      },
    })

    expect(result.config.active).toBe(true)
    expect(result.triggerId).toBeTruthy()

    ctx.set('triggerId', result.triggerId)
    ctx.set('taskIds', [])
  })

  // ─── INT-61: Send inbound message — agent starts ────────────────────

  test('INT-61: Send inbound message — agent starts', async ({ api }) => {
    const { response } = await sendWebhookMessage(
      api,
      ctx.get<string>('triggerId')!,
      'Say hello world. Keep it brief.',
    )

    expect(response.status).toBe(200)

    const items = await waitForLoopTasks(api, ctx.get<string>('triggerId')!, 1)
    const taskId = items[0].task.taskId
    ctx.set('taskId', taskId)
    ctx.set('taskIds', [taskId])
  })

  // ─── INT-62: Wait for agent completion ──────────────────────────────

  test('INT-62: Wait for agent completion', async ({ api }, testInfo) => {
    const { sandboxReady, agentIdle } = profile().timeouts
    testInfo.setTimeout(sandboxReady + agentIdle + 30_000)

    // The sandbox comes up before the agent can run in it. Asserted on its own
    // so that a sandbox that never provisions does not read as an agent that
    // never finished.
    const { ready } = await waitForSandboxReady(api, ctx.get<string>('taskId')!)
    expect(ready, 'the sandbox the webhook created never became ready').toBe(true)

    const completed = await waitForAgentComplete(api, ctx.get<string>('taskId')!, {
      timeout: agentIdle,
      onProgress: (msg) => progress(`  ${msg}`),
    })

    expect(completed).toBe(true)
  })

  // ─── INT-63: Callback server received outbound POST ─────────────────

  test('INT-63: Callback server received outbound POST', async () => {
    // The callback may have already arrived during agent completion,
    // or we may need to wait a moment for the event consumer to process it
    const callback = await callbackServer.waitForCallback(15_000)

    expect(callback.method).toBe('POST')
    expect(callback.headers['content-type']).toContain('application/json')
    expect(callback.body).toMatchObject({
      task_id: expect.any(String),
      text: expect.any(String),
      is_error: false,
    })

    // Verify the task_id matches our task
    const body = callback.body as Record<string, unknown>
    expect(body.task_id).toBe(ctx.get<string>('taskId'))
  })
})
