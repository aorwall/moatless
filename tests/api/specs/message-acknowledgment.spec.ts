/**
 * Chain: Message Acknowledgment — Same-Session Multi-Message
 *
 * Verifies that multiple messages sent while the agent is running are
 * injected into the same CLI session (via send_user_message with
 * MessagePriority::Next), not queued for separate turns.
 *
 * Key assertion: only 1 session_init event — all messages handled
 * in a single session.
 *
 * Needs an environment with agent credentials — skipped where there are none.
 *
 * Chain:
 *   TASK-01 → SANDBOX-01 → MSG-01 (session_init) → MSG-01.1 (turn held) →
 *   MSG-02 (send 2nd) → MSG-03 (send 3rd) → MSG-03.1 (release) →
 *   MSG-04 (wait idle) → MSG-05 (single session) →
 *   MSG-06 (verify files) → MSG-07 (all acknowledged) →
 *   MSG-08 (one turn) → MSG-09 (no errors)
 */

import { posix } from 'node:path'
import { findNamedFileWithContent } from '../../shared/content-helpers'
import type { MoatApi } from '../../shared/moat-api'
import { budget, missingCapability, profile } from '../../shared/profile'
import { waitForCondition } from '../../shared/wait-helpers'
import { ChainContext } from '../context/chain-context'
import { chainEvents } from '../context/chain-events'
import { expect, test } from '../fixtures/api-fixtures'
import { progress } from '../helpers/progress'
import { discardTask } from '../helpers/teardown'
import { createTaskWithMessage, waitForSandboxReady } from '../steps'

const agentReason = missingCapability('claudeCredentials')

/** The file the first turn waits for, beside `first.txt`; MSG-03.1 writes it. */
const GATE_FILE = 'first.release'

/** Whether there is a turn in flight right now for the follow-up to join. */
async function agentIsRunning(api: MoatApi, taskId: string): Promise<boolean> {
  const status = await api.getWorkspaceStatus(taskId)
  return status.agentStatus === 'running'
}

test.describe('Chain: Message Acknowledgment — Same-Session Multi-Message', () => {
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

  // ─── TASK-01: Create task with first message ────────────────────────

  test('TASK-01: Create task with first message', async ({ api }) => {
    // The point of the chain is what happens to a message that arrives while
    // the agent is working, so the first turn waits on a gate only MSG-03.1
    // opens. A fixed `sleep` raced a slow follow-up and skipped the claims.
    const { taskId, task } = await createTaskWithMessage(
      api,
      ctx.repositoryId!,
      "Create a file called first.txt with the content 'First Message Created'. " +
        `Then, in the directory holding first.txt, run \`until [ -f ${GATE_FILE} ]; do sleep 1; done\` ` +
        'in the foreground and wait for it to return, and finish.',
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

    ctx.sandboxStatus = status!
    progress('Sandbox ready')
  })

  // ─── MSG-01: Wait for session_init (session is alive) ───────────────

  test('MSG-01: Session starts for first message', async ({}, testInfo) => {
    testInfo.setTimeout(budget('agentIdle'))

    const sessionInit = await events().waitForEvent({
      type: 'agent',
      subType: 'session_init',
      taskId: ctx.taskId!,
      timeout: profile().timeouts.agentIdle,
    })

    expect(sessionInit).not.toBeNull()

    const payload = sessionInit!.payload as { session_id?: string }
    expect(payload.session_id).toBeTruthy()

    ctx.set('sessionId', payload.session_id)
    progress(`session_init received, session_id=${payload.session_id}`)
  })

  // ─── MSG-01.1: The first turn is held on its gate ───────────────────

  test('MSG-01.1: First turn waits on its gate', async ({ api }, testInfo) => {
    testInfo.setTimeout(budget('agentIdle'))

    const first: { path?: string } = {}
    const written = await waitForCondition(
      async () => {
        const match = await findNamedFileWithContent(
          api,
          ctx.taskId!,
          'first.txt',
          'First Message Created',
        )
        first.path = match?.path
        return first.path !== undefined
      },
      { timeout: profile().timeouts.agentIdle, interval: 2000 },
    )
    expect(written, 'first.txt never appeared').toBe(true)

    ctx.set('gatePath', posix.join(posix.dirname(first.path!), GATE_FILE))
    expect(await agentIsRunning(api, ctx.taskId!), 'the gate did not hold the turn open').toBe(true)
    progress(`First turn waiting on ${ctx.get<string>('gatePath')}`)
  })

  // ─── MSG-02: Send second message while agent is running ─────────────

  test('MSG-02: Send second message during active session', async ({ api }) => {
    const running = await agentIsRunning(api, ctx.taskId!)
    expect(running, 'the first turn ended before its gate opened').toBe(true)

    await api.sendMessage(
      ctx.taskId!,
      "Also create a file called second.txt with the content 'Second Message Created'",
    )

    progress(`Second message sent, agent running: ${running}`)
  })

  // ─── MSG-03: Send third message after brief delay ───────────────────

  test('MSG-03: Send third message during active session', async ({ api }) => {
    // Brief delay to space out the messages
    await new Promise((r) => setTimeout(r, 2000))

    const running = await agentIsRunning(api, ctx.taskId!)
    expect(running, 'the first turn ended before its gate opened').toBe(true)

    await api.sendMessage(
      ctx.taskId!,
      "Also create a file called third.txt with the content 'Third Message Created'",
    )

    progress(`Third message sent, agent running: ${running}`)
  })

  // ─── MSG-03.1: Release the first turn ───────────────────────────────

  test('MSG-03.1: Open the gate', async ({ api }) => {
    await api.writeFile(ctx.taskId!, ctx.get<string>('gatePath')!, 'go\n')
    progress('Gate opened')
  })

  // ─── MSG-04: Wait for agent to become idle ──────────────────────────

  test('MSG-04: Agent completes and becomes idle', async ({ api }, testInfo) => {
    testInfo.setTimeout(budget('agentIdle'))

    const settled = await waitForCondition(
      async () => {
        const s = await api.getWorkspaceStatus(ctx.taskId!)
        return s.agentStatus === 'idle' || s.agentStatus === null
      },
      { timeout: profile().timeouts.agentIdle, interval: 2000 },
    )

    expect(settled).toBe(true)

    const status = await api.getWorkspaceStatus(ctx.taskId!)
    progress(`Agent state: ${status.agentStatus}`)
  })

  // ─── MSG-05: Assert single session (1 session_init) ─────────────────

  test('MSG-05: All messages handled in single session', async () => {
    const sessionInitCount = events().countEvents('agent', 'session_init', ctx.taskId!)

    expect(sessionInitCount).toBe(1)

    progress(`session_init count: ${sessionInitCount} (single session confirmed)`)
  })

  // ─── MSG-06: Verify all three files were created ────────────────────

  for (const [scenario, fileName, content] of [
    ['MSG-06', 'first.txt', 'First Message Created'],
    ['MSG-06.1', 'second.txt', 'Second Message Created'],
    ['MSG-06.2', 'third.txt', 'Third Message Created'],
  ] as const) {
    test(`${scenario}: ${fileName} exists with what was asked for`, async ({ api }) => {
      const match = await findNamedFileWithContent(api, ctx.taskId!, fileName, content)

      expect(match, `${fileName} is missing, or does not say "${content}"`).not.toBeNull()
      progress(`${fileName} found in: ${match!.path}`)
    })
  }

  // ─── MSG-07: All messages acknowledged ──────────────────────────────

  test('MSG-07: All messages acknowledged', async ({ api }) => {
    const { messages } = await api.getTaskMessages(ctx.taskId!)

    const queuedMessages = messages.filter((m) => m.isQueued === true)

    expect(queuedMessages.length).toBe(0)

    progress(`All ${messages.length} messages acknowledged, 0 queued`)
  })

  // ─── MSG-08: The three messages belong to one turn ──────────────────

  test('MSG-08: All three messages belong to one turn', async ({ api }) => {
    const { messages } = await api.getTaskMessages(ctx.taskId!)

    // A turn is one agent run. The sandbox coalesced all three messages into a
    // single run, so they share a turn rather than each opening one — which is
    // the same claim as the single session_init, seen from the conversation.
    const turns = messages
      .filter((m) => m.role === 'user')
      .map((m) => m.turnNumber)
      .filter((turn): turn is number => turn != null)

    expect(turns, 'every user message should carry a turn number').toHaveLength(3)
    expect(new Set(turns).size, `messages spread over turns ${turns.join(', ')}`).toBe(1)
  })

  // ─── MSG-09: Nothing failed along the way ───────────────────────────

  test('MSG-09: No sandbox errors were reported', async () => {
    expect(events().countEvents('sandbox', 'error', ctx.taskId!)).toBe(0)

    // Dump event summary
    const summary = events().getEventSummary(ctx.taskId!)
    progress(`SSE event summary: ${JSON.stringify(summary, null, 2)}`)
  })
})
