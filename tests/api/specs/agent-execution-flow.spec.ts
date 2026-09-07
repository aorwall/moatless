/**
 * Chain 1: Agent Execution & Multi-Turn
 *
 * Happy-path agent execution: task creation, sandbox provisioning, agent
 * execution, file verification, and multi-turn conversation.
 *
 * Absorbed from core-lifecycle.spec.ts — drops SERVER-01 and SANDBOX-01.1/01.2
 * (covered by sandbox-lifecycle.spec.ts).
 *
 * Needs an environment with agent credentials — skipped where there are none.
 *
 * Chain:
 *   TASK-01 → SANDBOX-01 → AGENT-01 → AGENT-01.1 → AGENT-01.2 →
 *   AGENT-01.3 → FILE-06 →
 *   AGENT-02 → AGENT-02.1 → AGENT-02.2 → AGENT-02.3
 */

import { findNamedFileWithContent, flattenTree } from '../../shared/content-helpers'
import { budget, missingCapability } from '../../shared/profile'
import { waitForCondition } from '../../shared/wait-helpers'
import { ChainContext } from '../context/chain-context'
import { chainEvents } from '../context/chain-events'
import { expect, test } from '../fixtures/api-fixtures'
import { progress } from '../helpers/progress'
import { discardTask } from '../helpers/teardown'
import { createTaskWithMessage, waitForAgentTurnComplete, waitForSandboxReady } from '../steps'

const agentReason = missingCapability('agentCredentials')

test.describe('Chain: Agent Execution & Multi-Turn', () => {
  test.describe.configure({ mode: 'serial' })

  test.skip(agentReason !== null, agentReason ?? '')

  const ctx = new ChainContext()
  const events = chainEvents()

  test.beforeAll(async ({ api }) => {
    const repo = await api.ensureTemplateRepository()
    ctx.repositoryId = repo.id
    ctx.repository = repo
  })

  test.afterAll(async ({ api }) => {
    await discardTask(api, ctx.taskId)
  })

  // ─── TASK-01: Create task with message ──────────────────────────────

  test('TASK-01: Create task with message', async ({ api }) => {
    const { taskId, task } = await createTaskWithMessage(
      api,
      ctx.repositoryId!,
      // Named rather than "the main page": the suite runs against whatever
      // repository the environment provides, and only the file the message
      // names is something every one of them can be checked for.
      "Create a file called e2e-marker.txt containing exactly 'E2E Test Success'",
    )

    expect(task.status).toBe('open')
    expect(taskId).toBeTruthy()

    ctx.taskId = taskId
    ctx.task = task

    progress(`Task created: ${taskId}`)
  })

  // ─── SANDBOX-01: Sandbox provisions automatically ───────────────────

  test('SANDBOX-01: Sandbox provisions automatically', async ({ api }, testInfo) => {
    testInfo.setTimeout(budget('sandboxReady'))

    const { ready, status } = await waitForSandboxReady(api, ctx.taskId!)

    expect(ready).toBe(true)
    expect(status).not.toBeNull()
    expect(status!.sandboxStatus).toBe('ready')

    ctx.sandboxStatus = status!
    progress('Sandbox ready')
  })

  // ─── AGENT-01: Agent executes initial message ───────────────────────

  test('AGENT-01: Agent executes initial message', async ({ api }, testInfo) => {
    testInfo.setTimeout(budget('agentIdle'))

    const { completed } = await waitForAgentTurnComplete(events(), api, ctx.taskId!, 1)

    if (!completed) {
      // Diagnostic: dump what we actually received
      const summary = events().getEventSummary(ctx.taskId!)
      progress(`SSE event summary: ${JSON.stringify(summary, null, 2)}`)

      const status = await api.getWorkspaceStatus(ctx.taskId!)
      progress(`Workspace: env=${status.sandboxStatus}, agent=${status.agentStatus}`)

      const taskEvents = events().getEventsForTask(ctx.taskId!)
      progress(`Total events for task: ${taskEvents.length}`)
      for (const e of taskEvents.slice(-10)) {
        progress(`  ${e.type}.${e.subType || '*'} @ ${e.timestamp || '?'}`)
      }
    }

    expect(completed).toBe(true)
    ctx.turnCount = 1

    progress('Agent turn 1 completed')
  })

  // ─── AGENT-01.1: Agent state transitions ────────────────────────────

  test('AGENT-01.1: Agent state transitions', async ({ api }) => {
    // After turn completes, agent state may still be transitioning.
    // Poll briefly (up to 10s) for it to settle to idle/none.
    const settled = await waitForCondition(
      async () => {
        const s = await api.getWorkspaceStatus(ctx.taskId!)
        return s.agentStatus === 'idle' || s.agentStatus === null
      },
      { timeout: 10_000, interval: 1000 },
    )

    const status = await api.getWorkspaceStatus(ctx.taskId!)
    if (!settled) {
      progress(`State did not settle: agentStatus=${status.agentStatus}`)
    }
    expect(['idle', null]).toContain(status.agentStatus)
  })

  // ─── AGENT-01.2: SSE agent lifecycle events ─────────────────────────

  test('AGENT-01.2: SSE agent lifecycle events', async () => {
    const taskEvents = events().getEventsForTask(ctx.taskId!)

    const hasAgentStarted = taskEvents.some((e) => e.type === 'agent' && e.subType === 'started')
    const hasAgentMessage = taskEvents.some((e) => e.type === 'agent' && e.subType === 'message')
    const hasAgentCompleted = taskEvents.some(
      (e) => e.type === 'agent' && e.subType === 'completed',
    )
    const hasTurnActive = taskEvents.some((e) => e.type === 'taskTurn' && e.subType === 'active')
    const hasTurnCompleted = taskEvents.some(
      (e) => e.type === 'taskTurn' && e.subType === 'completed',
    )

    expect(hasAgentStarted).toBe(true)
    expect(hasAgentMessage).toBe(true)
    expect(hasAgentCompleted).toBe(true)
    expect(hasTurnActive).toBe(true)
    expect(hasTurnCompleted).toBe(true)
  })

  // ─── AGENT-01.3: Messages endpoint returns conversation ─────────────

  test('AGENT-01.3: Messages endpoint returns conversation', async ({ api }) => {
    const { messages } = await api.getTaskMessages(ctx.taskId!)

    expect(messages.length).toBeGreaterThanOrEqual(2)

    const hasUser = messages.some((m) => m.role === 'user')
    const hasAssistant = messages.some((m) => m.role === 'assistant')

    expect(hasUser).toBe(true)
    expect(hasAssistant).toBe(true)
  })

  // ─── FILE-06: Files reflect agent changes ───────────────────────────

  test('FILE-06: Files reflect agent changes', async ({ api }) => {
    const match = await findNamedFileWithContent(
      api,
      ctx.taskId!,
      'e2e-marker.txt',
      'E2E Test Success',
    )

    expect(match, 'e2e-marker.txt is missing, or does not say what was asked').not.toBeNull()
    progress(`Content "E2E Test Success" found in: ${match!.path}`)

    // The tree also reports what git considers changed since checkout, which is
    // how the UI marks the agent's work. A file on disk that the tree does not
    // call a change would never show up there.
    const tree = await api.getFileTree(ctx.taskId!, '.', true)
    const node = flattenTree(tree.tree).find((entry) => entry.path === match!.path)
    expect(node, `${match!.path} is missing from the file tree`).toBeDefined()
    expect(node?.isChanged, "the agent's new file is not marked as changed").toBe(true)
  })

  // ─── AGENT-02: Multi-turn conversation ──────────────────────────────

  test('AGENT-02: Multi-turn conversation', async ({ api }, testInfo) => {
    testInfo.setTimeout(budget('agentIdle'))

    await api.sendMessage(ctx.taskId!, "Change e2e-marker.txt to say 'Multi-Turn Success' instead")

    progress('Follow-up message sent')

    const { completed } = await waitForAgentTurnComplete(events(), api, ctx.taskId!, 2)

    expect(completed).toBe(true)
    ctx.turnCount = 2

    progress('Agent turn 2 completed')
  })

  // ─── AGENT-02.1: Agent re-activates on follow-up ────────────────────

  test('AGENT-02.1: Agent re-activates on follow-up', async ({ api }) => {
    // After second turn, agent state may still be transitioning.
    await waitForCondition(
      async () => {
        const s = await api.getWorkspaceStatus(ctx.taskId!)
        return s.agentStatus === 'idle' || s.agentStatus === null
      },
      { timeout: 10_000, interval: 1000 },
    )

    const status = await api.getWorkspaceStatus(ctx.taskId!)
    expect(['idle', null]).toContain(status.agentStatus)
  })

  // ─── AGENT-02.2: Second taskTurn.completed event ────────────────────

  test('AGENT-02.2: Second taskTurn.completed event', async () => {
    const agentCount = events().countEvents('agent', 'completed', ctx.taskId!)
    const turnCount = events().countEvents('taskTurn', 'completed', ctx.taskId!)

    expect(agentCount).toBeGreaterThanOrEqual(2)
    expect(turnCount).toBeGreaterThanOrEqual(2)

    progress(`Events: agent.completed=${agentCount}, taskTurn.completed=${turnCount}`)
  })

  // ─── AGENT-02.3: Files reflect second-turn changes ──────────────────

  test('AGENT-02.3: Files reflect second-turn changes', async ({ api }) => {
    const match = await findNamedFileWithContent(
      api,
      ctx.taskId!,
      'e2e-marker.txt',
      'Multi-Turn Success',
    )

    expect(match, 'the second turn did not change e2e-marker.txt').not.toBeNull()
    progress(`Content "Multi-Turn Success" found in: ${match!.path}`)
  })
})
