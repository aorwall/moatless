/**
 * Chain: Codex Execution & Multi-Turn
 *
 * The Claude chain in agent-execution-flow.spec.ts, run with the Codex agent:
 * a task, its sandbox, two turns, and the file each turn asked for.
 *
 * Chain:
 *   CODEX-01 → CODEX-02 → CODEX-03 → CODEX-04 → CODEX-05
 */

import { findNamedFileWithContent } from '../../shared/content-helpers'
import { budget, missingCapability } from '../../shared/profile'
import { ChainContext } from '../context/chain-context'
import { chainEvents } from '../context/chain-events'
import { expect, test } from '../fixtures/api-fixtures'
import { progress } from '../helpers/progress'
import { discardTask } from '../helpers/teardown'
import { waitForAgentTurnComplete, waitForSandboxReady } from '../steps'

const reason = missingCapability('codexConnection')

test.describe('Chain: Codex Execution & Multi-Turn', () => {
  test.describe.configure({ mode: 'serial' })

  test.skip(reason !== null, reason ?? '')

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

  // ─── CODEX-01: Create a Codex task with message ─────────────────────

  test('CODEX-01: Create a Codex task with message', async ({ api }) => {
    const task = await api.createVibeTask({
      templateId: ctx.repositoryId!,
      name: 'codex-task',
      message: "Create a file called codex-marker.txt containing exactly 'Codex Test Success'",
      agentType: 'codex',
    })

    expect(task.status, 'a new Codex task is not open').toBe('open')
    ctx.taskId = task.taskId
    ctx.task = task
    progress(`Task created: ${task.taskId}`)
  })

  // ─── CODEX-02: Sandbox provisions ───────────────────────────────────

  test('CODEX-02: Sandbox provisions', async ({ api }, testInfo) => {
    testInfo.setTimeout(budget('sandboxReady'))

    const { ready, status } = await waitForSandboxReady(api, ctx.taskId!)

    expect(ready, `the sandbox never became ready (last: ${status?.sandboxStatus})`).toBe(true)
    progress('Sandbox ready')
  })

  // ─── CODEX-03: Codex completes the first turn ───────────────────────

  test('CODEX-03: Codex completes the first turn', async ({ api }, testInfo) => {
    testInfo.setTimeout(budget('agentIdle'))

    const { completed } = await waitForAgentTurnComplete(events(), api, ctx.taskId!, 1)

    expect(completed, 'Codex did not finish its first turn').toBe(true)
    const match = await findNamedFileWithContent(
      api,
      ctx.taskId!,
      'codex-marker.txt',
      'Codex Test Success',
    )
    expect(match, 'codex-marker.txt is missing, or does not say what was asked').not.toBeNull()
  })

  // ─── CODEX-04: Codex completes a follow-up turn ─────────────────────

  test('CODEX-04: Codex completes a follow-up turn', async ({ api }, testInfo) => {
    testInfo.setTimeout(budget('agentIdle'))

    await api.sendMessage(ctx.taskId!, "Change codex-marker.txt to say 'Codex Multi-Turn' instead")
    const { completed } = await waitForAgentTurnComplete(events(), api, ctx.taskId!, 2)

    expect(completed, 'Codex did not finish the follow-up turn').toBe(true)
    const match = await findNamedFileWithContent(
      api,
      ctx.taskId!,
      'codex-marker.txt',
      'Codex Multi-Turn',
    )
    expect(match, 'the follow-up turn did not change codex-marker.txt').not.toBeNull()
  })

  // ─── CODEX-05: The conversation holds both turns ────────────────────

  test('CODEX-05: The conversation holds both turns', async ({ api }) => {
    const { messages } = await api.getTaskMessages(ctx.taskId!)

    const users = messages.filter((m) => m.role === 'user').length
    const assistants = messages.filter((m) => m.role === 'assistant').length
    expect(users, 'the messages endpoint lost a user message').toBeGreaterThanOrEqual(2)
    expect(assistants, 'the messages endpoint has no Codex reply').toBeGreaterThanOrEqual(1)
  })
})
