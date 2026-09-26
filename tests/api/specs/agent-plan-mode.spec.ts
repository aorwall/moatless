/**
 * Chain 3: Plan Mode
 *
 * Agent execution in plan mode: the agent produces a plan for user approval
 * before making changes.
 *
 * Needs an environment with agent credentials — skipped where there are none.
 *
 * Chain:
 *   TASK-01 (plan mode) → SANDBOX-01 → AGENT-03 →
 *   AGENT-03.1 → AGENT-03.2
 */

import { budget, missingCapability, profile } from '../../shared/profile'
import type { GetMessagesResponse } from '../../shared/types'
import { ChainContext } from '../context/chain-context'
import { chainEvents } from '../context/chain-events'
import { expect, test } from '../fixtures/api-fixtures'
import { progress } from '../helpers/progress'
import { discardTask } from '../helpers/teardown'
import { waitForPendingToolCall, waitForSandboxReady } from '../steps'

const agentReason = missingCapability('claudeCredentials')

/** The plan the agent put up for approval, as one particular response carries it. */
function planOf(messages: GetMessagesResponse, toolUseId: string): string | undefined {
  const block = messages.messages
    .flatMap((message) => message.content)
    .find((content) => content.toolUseId === toolUseId)
  const input = block?.toolInput as { plan?: string } | undefined
  return input?.plan
}

test.describe('Chain: Plan Mode', () => {
  test.describe.configure({ mode: 'serial' })

  test.skip(agentReason !== null, agentReason ?? '')

  const ctx = new ChainContext()
  const events = chainEvents()

  test.beforeAll(async ({ api }) => {
    const repo = await api.ensureTemplateRepository()
    ctx.repositoryId = repo.id
  })

  test.afterAll(async ({ api }) => {
    await discardTask(api, ctx.taskId)
  })

  // ─── TASK-01: Create task in plan mode ──────────────────────────────

  test('TASK-01: Create task in plan mode', async ({ api }) => {
    const task = await api.createVibeTask({
      templateId: ctx.repositoryId!,
      name: 'plan mode',
      message: "Create a file called plan-mode.txt containing exactly 'Plan Mode Test'.",
      agentMode: 'plan',
      // Someone approves the plan further down the chain, so this task is
      // watched — a headless one gets no plan tools.
      headless: false,
    })

    expect(task.status).toBe('open')

    ctx.taskId = task.taskId
    ctx.task = task

    progress(`Task created: ${task.taskId}`)
  })

  // ─── SANDBOX-01: Sandbox provisions ─────────────────────────────────

  test('SANDBOX-01: Sandbox provisions', async ({ api }, testInfo) => {
    testInfo.setTimeout(budget('sandboxReady'))

    const { ready } = await waitForSandboxReady(api, ctx.taskId!)

    expect(ready).toBe(true)

    // The task record does not carry the mode, but the message that started
    // the turn does. Asking for plan mode and silently getting edit mode would
    // leave the whole chain passing while testing nothing.
    const { messages } = await api.getTaskMessages(ctx.taskId!)
    const opening = messages.find((message) => message.role === 'user')
    expect(opening?.agentMode, 'the opening message did not run in plan mode').toBe('plan')

    progress('Sandbox ready')
  })

  // ─── AGENT-03: Plan mode flow ───────────────────────────────────────

  test('AGENT-03: Plan mode flow', async ({ api }, testInfo) => {
    testInfo.setTimeout(budget('agentIdle'))

    // Two acceptable endings: the agent puts a plan up for approval, or it
    // decides it has nothing to plan and finishes.
    const outcome = await waitForPendingToolCall(
      events(),
      api,
      ctx.taskId!,
      'ExitPlanMode',
      profile().timeouts.agentIdle,
    )
    expect(
      outcome.toolUseId !== null || outcome.completed,
      'the agent neither put a plan up nor finished its turn',
    ).toBe(true)

    ctx.set('planModeActive', outcome.toolUseId !== null)
    if (outcome.toolUseId) {
      ctx.set('toolUseId', outcome.toolUseId)
      progress('Agent paused for approval')
    } else {
      progress('Agent completed without plan pause')
    }
  })

  // ─── AGENT-03.1: Agent produces plan for approval ───────────────────

  test('AGENT-03.1: Agent produces plan for approval', async ({ api }) => {
    const messages = await api.getTaskMessages(ctx.taskId!)

    expect(messages.messages.length).toBeGreaterThanOrEqual(2)
    expect(messages.messages.some((m) => m.role === 'assistant')).toBe(true)

    if (!ctx.get('planModeActive')) {
      return
    }

    const toolUseId = ctx.get<string>('toolUseId')!
    const plan = planOf(messages, toolUseId)
    expect(plan, 'the plan the agent proposed is empty').toBeTruthy()

    // The chat list always asks for the summary, and the approval card renders
    // the plan out of the tool call — so a projection that drops it leaves the
    // card blank while the full response still looks right.
    const summary = await api.getTaskMessagesSummary(ctx.taskId!)
    expect(
      planOf(summary, toolUseId),
      'the summary projection dropped the plan the approval card renders',
    ).toBe(plan)
  })

  // ─── AGENT-03.2: User approves, agent executes ──────────────────────

  test('AGENT-03.2: User approves, agent executes', async ({ api }, testInfo) => {
    testInfo.setTimeout(budget('agentIdle'))

    if (ctx.get('planModeActive')) {
      const turnsBefore = events().countEvents('taskTurn', 'completed', ctx.taskId!)

      // Answered the way the chat does it: the approval is a response to the
      // call the agent is blocked on, not a new message beside it.
      await api.sendToolResponse(ctx.taskId!, {
        toolName: 'ExitPlanMode',
        toolUseId: ctx.get<string>('toolUseId')!,
        toolResponse: { approved: true },
      })

      const completed = await events().waitForNthEvent(
        'taskTurn',
        'completed',
        ctx.taskId!,
        turnsBefore + 1,
        profile().timeouts.agentIdle,
      )
      expect(completed, 'the agent never finished after the plan was approved').not.toBeNull()
    }

    const turnCount = events().countEvents('taskTurn', 'completed', ctx.taskId!)
    expect(turnCount).toBeGreaterThanOrEqual(1)

    progress('Agent execution completed')
  })
})
