/**
 * Standalone: Sandbox replacement ordering
 *
 * A message sent while the sandbox is down is persisted, and the replacement
 * sandbox is provisioned to answer it. The ordering is the contract: the
 * replacement must restore the conversation before it accepts that message, or
 * the agent answers with no memory of what came before.
 *
 * Run once per harness, because each restores its session its own way and only
 * a harness that actually produced output proves anything.
 *
 * Scenarios:
 *   AGENT-09.1: A replaced sandbox resumes the Claude session before answering
 *   AGENT-09.2: A replaced sandbox resumes the Codex session before answering
 */

import { EventStream } from '../../shared/event-stream'
import type { MoatApi } from '../../shared/moat-api'
import { missingCapability, profile } from '../../shared/profile'
import { expect, test } from '../fixtures/api-fixtures'
import { stopSandbox, waitForSandboxReady } from '../steps'

interface Harness {
  scenario: string
  agentType: string
  model: string
  /** What the provider reports on a message it produced. */
  modelPrefix: string
}

const HARNESSES: Harness[] = [
  {
    scenario: 'AGENT-09.1',
    agentType: 'claude-code-cli',
    model: 'haiku',
    modelPrefix: 'claude-',
  },
  {
    scenario: 'AGENT-09.2',
    agentType: 'codex',
    model: 'gpt-5.4-mini',
    modelPrefix: 'gpt-',
  },
]

/**
 * The session id the harness announced on its Nth start.
 *
 * The id is what proves continuity: a replacement that opened a new session
 * would answer the pending message with none of the conversation behind it.
 */
async function sessionIdOfStart(
  sse: EventStream,
  taskId: string,
  nth: number,
  timeout: number,
): Promise<string> {
  const event = await sse.waitForNthEvent('agent', 'session_init', taskId, nth, timeout)
  expect(event, `the harness did not open session ${nth}`).not.toBeNull()
  const sessionId = event?.payload.session_id
  expect(typeof sessionId, 'session_init must carry a session id').toBe('string')
  return sessionId as string
}

/** Assert the harness under test is the one that produced the output. */
async function expectModelFamily(
  api: MoatApi,
  taskId: string,
  harness: Harness,
  turnNumber?: number,
): Promise<void> {
  const { messages } = await api.getTaskMessages(taskId)
  const produced = messages.some(
    (message) =>
      message.role === 'assistant' &&
      (turnNumber === undefined || message.turnNumber === turnNumber) &&
      message.model?.startsWith(harness.modelPrefix),
  )
  expect(produced, `${harness.agentType} output must carry a ${harness.modelPrefix} model`).toBe(
    true,
  )
}

for (const harness of HARNESSES) {
  test.describe(`Standalone: Sandbox replacement ordering (${harness.agentType})`, () => {
    const reason = missingCapability('agentCredentials')
    test.skip(reason !== null, reason ?? '')

    test(`${harness.scenario}: A replaced sandbox resumes the session before answering`, async ({
      api,
      driver,
    }, testInfo) => {
      // Two sandbox provisions and two agent turns, end to end.
      testInfo.setTimeout(profile().timeouts.sandboxReady * 2 + profile().timeouts.agentIdle * 2)

      const sse = new EventStream(driver)
      await sse.start()

      const repo = await api.ensureTemplateRepository()
      const task = await api.createVibeTask({
        templateId: repo.id,
        name: `${harness.agentType} replacement ordering`,
        message: 'Reply with exactly FIRST and do nothing else.',
        agentType: harness.agentType,
        model: harness.model,
      })

      try {
        const { ready } = await waitForSandboxReady(
          api,
          task.taskId,
          profile().timeouts.sandboxReady,
        )
        expect(ready, 'the first sandbox never became ready').toBe(true)

        const firstSession = await sessionIdOfStart(
          sse,
          task.taskId,
          1,
          profile().timeouts.agentIdle,
        )

        const firstTurn = await sse.waitForNthEvent(
          'taskTurn',
          'completed',
          task.taskId,
          1,
          profile().timeouts.agentIdle,
        )
        expect(firstTurn, 'the first turn never completed').not.toBeNull()
        await expectModelFamily(api, task.taskId, harness)

        const startsBefore = sse.countEvents('agent', 'session_init', task.taskId)
        const turnsBefore = sse.countEvents('taskTurn', 'completed', task.taskId)

        const stopped = await stopSandbox(api, task.taskId)
        expect(stopped, 'the sandbox never stopped').not.toBeNull()

        // Sent with nothing running to receive it. The backend persists it
        // and provisions a replacement to answer it.
        const pending = await api.postMessage(task.taskId, {
          message: 'Reply with exactly SECOND and do nothing else.',
          model: harness.model,
          agentMode: 'code',
          startFresh: false,
        })
        expect(pending.uuid, 'a queued message must be persisted').toBeTruthy()

        const replaced = await waitForSandboxReady(
          api,
          task.taskId,
          profile().timeouts.sandboxReady,
        )
        expect(replaced.ready, 'the replacement sandbox never became ready').toBe(true)

        // The contract: same session, not a fresh one.
        const resumedSession = await sessionIdOfStart(
          sse,
          task.taskId,
          startsBefore + 1,
          profile().timeouts.agentIdle,
        )
        expect(resumedSession).toBe(firstSession)

        const secondTurn = await sse.waitForNthEvent(
          'taskTurn',
          'completed',
          task.taskId,
          turnsBefore + 1,
          profile().timeouts.agentIdle,
        )
        expect(secondTurn, 'the replacement turn never completed').not.toBeNull()

        const { messages } = await api.getTaskMessages(task.taskId)
        const accepted = messages.find((message) => message.uuid === pending.uuid)
        expect(accepted, 'the pending message vanished').toBeDefined()
        expect(accepted?.sessionId).toBe(firstSession)
        // Acknowledged — a message still queued after the turn would be
        // answered again by the next one. The flag is reported only while
        // a message is waiting, so acknowledged means absent, not `false`.
        expect(accepted?.isQueued ?? false).toBe(false)

        await expectModelFamily(api, task.taskId, harness, accepted?.turnNumber ?? undefined)

        const reread = await api.getTask(task.taskId)
        expect(reread.agentType).toBe(harness.agentType)
      } finally {
        sse.stop()
        await api.stopTask(task.taskId).catch(() => {})
        await api.deleteTask(task.taskId).catch(() => {})
      }
    })
  })
}
