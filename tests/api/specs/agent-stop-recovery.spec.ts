/**
 * Chain 3: Agent Stop, Resume & Recovery
 *
 * Stopping an agent mid-execution, resuming after a sandbox restart,
 * and recovering from agent errors.
 *
 * Requires Claude authentication (CLAUDE_CODE_OAUTH_TOKEN or ANTHROPIC_API_KEY).
 *
 * Chain:
 *   TASK-01 → AGENT-04 → AGENT-04.1 → AGENT-04.2 →
 *   AGENT-06.1 → AGENT-06.2 → AGENT-06.3 → AGENT-06.4 → AGENT-06.5 →
 *   AGENT-05 (skipped) → AGENT-05.1 (skipped) → AGENT-05.2 (skipped)
 */

import { findNamedFileWithContent } from '../../shared/content-helpers'
import { budget, missingCapability, profile } from '../../shared/profile'
import { ChainContext } from '../context/chain-context'
import { chainEvents } from '../context/chain-events'
import { expect, test } from '../fixtures/api-fixtures'
import { progress } from '../helpers/progress'
import { discardTask } from '../helpers/teardown'
import { createTaskWithMessage, startSandbox, stopSandbox, waitForSandboxReady } from '../steps'

const agentReason = missingCapability('claudeCredentials')

test.describe('Chain: Agent Stop, Resume & Recovery', () => {
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

  // ─── TASK-01: Create task with message (agent starts) ───────────────

  test('TASK-01: Create task with message (agent starts)', async ({ api }, testInfo) => {
    testInfo.setTimeout(budget('sandboxReady'))

    const { taskId, task } = await createTaskWithMessage(
      api,
      ctx.repositoryId!,
      'Create a complex multi-page application with a home page, about page, and contact page. Take your time to plan this carefully.',
    )

    expect(task.status).toBe('open')
    ctx.taskId = taskId
    ctx.task = task

    progress(`Task created: ${taskId}`)

    // Wait for sandbox to be ready
    const { ready } = await waitForSandboxReady(api, taskId)
    expect(ready).toBe(true)
    progress('Sandbox ready')

    // Verify sandbox SSE events arrived
    const sandboxEvents = events()
      .getEventsForTask(taskId)
      .filter((e) => e.type === 'sandbox' || e.type === 'environment')
    progress(`Sandbox SSE events: ${sandboxEvents.length}`)
    for (const e of sandboxEvents) {
      progress(`  ${e.type}.${e.subType || '*'}`)
    }

    // Check workspace status for agent state
    const wsStatus = await api.getWorkspaceStatus(taskId)
    progress(`Workspace: env=${wsStatus.sandboxStatus}, agent=${wsStatus.agentStatus}`)

    // Wait for agent to start running
    const agentStarted = await events().waitForEvent({
      type: 'agent',
      subType: 'started',
      taskId,
      timeout: profile().timeouts.agentIdle,
    })

    if (!agentStarted) {
      // Diagnostic: dump all events received
      const summary = events().getEventSummary(taskId)
      progress(`SSE event summary: ${JSON.stringify(summary, null, 2)}`)

      const allEvents = events().getEventsForTask(taskId)
      progress(`Total events for task: ${allEvents.length}`)
      for (const e of allEvents) {
        progress(`  ${e.type}.${e.subType || '*'} @ ${e.timestamp || '?'}`)
      }

      // Also check if agent is already running via API
      const currentStatus = await api.getWorkspaceStatus(taskId)
      progress(`Current agent state via API: ${currentStatus.agentStatus}`)
    }

    expect(agentStarted).not.toBeNull()
    progress('Agent started running')
  })

  // ─── AGENT-04: Stop agent mid-execution ─────────────────────────────

  test('AGENT-04: Stop agent mid-execution', async ({ api }, testInfo) => {
    testInfo.setTimeout(budget())

    // Stop the running agent
    await api.stopTask(ctx.taskId!)

    // Wait for agent to become idle
    const deadline = Date.now() + 30_000
    let isIdle = false

    while (Date.now() < deadline) {
      const status = await api.getWorkspaceStatus(ctx.taskId!)
      if (status.agentStatus === 'idle' || status.agentStatus === null) {
        isIdle = true
        break
      }
      await new Promise((r) => setTimeout(r, 1000))
    }

    expect(isIdle).toBe(true)
    progress('Agent stopped')
  })

  // ─── AGENT-04.1: Sandbox remains ready ──────────────────────────────

  test('AGENT-04.1: Sandbox remains ready or stopped', async ({ api }) => {
    const status = await api.getWorkspaceStatus(ctx.taskId!)

    // In K8s mode, the sandbox pod persists after agent stop (ready).
    // In Docker mode, stopping the agent also stops the container (stopped).
    expect(['ready', 'stopped']).toContain(status.sandboxStatus)

    ctx.set('sandboxStatusAfterAgentStop', status.sandboxStatus)
    progress(`Sandbox ${status.sandboxStatus} after agent stop`)
  })

  // ─── AGENT-04.2: SSE agent.stopped event ────────────────────────────

  test('AGENT-04.2: SSE agent.stopped event', async () => {
    const taskEvents = events().getEventsForTask(ctx.taskId!)

    const hasAgentStopped = taskEvents.some((e) => e.type === 'agent' && e.subType === 'stopped')
    const hasTurnStopped = taskEvents.some((e) => e.type === 'taskTurn' && e.subType === 'stopped')

    expect(hasAgentStopped || hasTurnStopped).toBe(true)

    progress(`Events: agent.stopped=${hasAgentStopped}, taskTurn.stopped=${hasTurnStopped}`)
  })

  // ─── AGENT-06.1: Sandbox stops ──────────────────────────────────────

  test('AGENT-06.1: Sandbox stops', async ({ api }, testInfo) => {
    testInfo.setTimeout(budget('sandboxTeardown'))

    // Stopping the agent may already have taken the sandbox with it. What the
    // scenario claims is where the sandbox ends up, not how it got there, so
    // either way the state is asserted rather than the test returning early.
    const before = await api.getWorkspaceStatus(ctx.taskId!)
    const status = before.sandboxStatus === 'stopped' ? before : await stopSandbox(api, ctx.taskId!)

    expect(status).not.toBeNull()
    expect(status!.sandboxStatus).toBe('stopped')

    progress(`Sandbox stopped (${before.sandboxStatus === 'stopped' ? 'already' : 'on request'})`)
  })

  // ─── AGENT-06.2: Sandbox restarts with files intact ─────────────────

  test('AGENT-06.2: Sandbox restarts with files intact', async ({ api }, testInfo) => {
    testInfo.setTimeout(budget('sandboxReady'))

    const { ready } = await startSandbox(api, ctx.taskId!)
    expect(ready).toBe(true)

    const files = await api.listWorkspaceFiles(ctx.taskId!, '.', false)
    expect(files.length).toBeGreaterThan(0)

    progress(`Sandbox restarted, ${files.length} files present`)
  })

  // ─── AGENT-06.3: Agent resumes with follow-up message ───────────────

  test('AGENT-06.3: Agent resumes with follow-up message', async ({ api }, testInfo) => {
    testInfo.setTimeout(budget('agentIdle'))

    const turnsBefore = events().countEvents('taskTurn', 'completed', ctx.taskId!)

    await api.sendMessage(
      ctx.taskId!,
      // Named, so the assertion holds against whatever repository the
      // environment provides.
      "Create a file called resumed.txt containing exactly 'Resumed Session Works'",
    )

    progress('Follow-up message sent after restart')

    const event = await events().waitForNthEvent(
      'taskTurn',
      'completed',
      ctx.taskId!,
      turnsBefore + 1,
      profile().timeouts.agentIdle,
    )

    expect(event).not.toBeNull()
    progress('Agent completed resumed turn')
  })

  // ─── AGENT-06.4: Agent retains conversation history ─────────────────

  test('AGENT-06.4: Agent retains conversation history', async ({ api }) => {
    const { messages } = await api.getTaskMessages(ctx.taskId!)

    // Should have at least: original user msg, agent response, follow-up msg, agent response
    expect(messages.length).toBeGreaterThanOrEqual(4)

    const hasUser = messages.some((m) => m.role === 'user')
    const hasAssistant = messages.some((m) => m.role === 'assistant')
    expect(hasUser).toBe(true)
    expect(hasAssistant).toBe(true)

    // Verify messages from both before and after restart
    const userMessages = messages.filter((m) => m.role === 'user')
    expect(userMessages.length).toBeGreaterThanOrEqual(2)

    progress(
      `Conversation history: ${messages.length} messages, ${userMessages.length} user messages`,
    )
  })

  // ─── AGENT-06.5: Files reflect resumed-session changes ──────────────

  test('AGENT-06.5: Files reflect resumed-session changes', async ({ api }) => {
    const match = await findNamedFileWithContent(
      api,
      ctx.taskId!,
      'resumed.txt',
      'Resumed Session Works',
    )

    expect(match, 'the resumed session left nothing behind').not.toBeNull()
    progress(`Content "Resumed Session Works" found in: ${match!.path}`)
  })
})
