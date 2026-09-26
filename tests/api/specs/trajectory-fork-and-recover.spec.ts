/**
 * Standalone: Trajectory fork and recover
 *
 * The harness session file is written back from the Postgres trajectory rather than from `transcripts.tar`. A fork must see
 * only the history up to its cut, and a replaced pod must see all of it.
 *
 * Run once per harness, because each writes its session file in its own format.
 *
 * Scenarios:
 *   TRAJ-01.1: A Claude fork and a replaced Claude pod answer from the right history
 *   TRAJ-01.2: A Codex fork and a replaced Codex pod answer from the right history
 */

import { EventStream } from '../../shared/event-stream'
import type { MoatApi } from '../../shared/moat-api'
import { type Capabilities, missingCapability, profile } from '../../shared/profile'
import { expect, test } from '../fixtures/api-fixtures'
import { stopSandbox, waitForSandboxReady } from '../steps'

interface Harness {
  scenario: string
  agentProfile: string
  /** What the run needs before this harness can start. */
  credentials: keyof Capabilities
  model: string
}

const HARNESSES: Harness[] = [
  {
    scenario: 'TRAJ-01.1',
    agentProfile: 'claude-code-cli',
    credentials: 'claudeCredentials',
    model: 'haiku',
  },
  {
    scenario: 'TRAJ-01.2',
    agentProfile: 'codex',
    credentials: 'codexConnection',
    model: 'gpt-5.5',
  },
]

const FIRST_NAME = 'PERIWINKLE'
const SECOND_NAME = 'SAFFRON'
const QUESTION =
  'List every release name I have told you so far, separated by commas. Use no tools.'

/** The text of the task's own assistant messages in its latest Turn. */
async function ownAnswers(api: MoatApi, taskId: string): Promise<string> {
  const { messages } = await api.getTaskMessages(taskId)
  const own = messages.filter((message) => message.role === 'assistant' && !message.sourceTaskId)
  const lastTurn = Math.max(...own.map((message) => message.turnNumber ?? 0))
  return own
    .filter((message) => (message.turnNumber ?? 0) === lastTurn)
    .flatMap((message) => message.content)
    .map((block) => (block.type === 'text' ? block.text : ''))
    .join('\n')
}

async function waitForTurns(sse: EventStream, taskId: string, turns: number): Promise<void> {
  const event = await sse.waitForNthEvent(
    'taskTurn',
    'completed',
    taskId,
    turns,
    profile().timeouts.agentIdle,
  )
  expect(event, `task ${taskId} never completed Turn ${turns}`).not.toBeNull()
}

for (const harness of HARNESSES) {
  test.describe(`Standalone: Trajectory fork and recover (${harness.agentProfile})`, () => {
    const reason = missingCapability(harness.credentials)
    test.skip(reason !== null, reason ?? '')

    test(`${harness.scenario}: A fork and a replaced pod answer from the right history`, async ({
      api,
      driver,
    }, testInfo) => {
      // Three provisions and five Turns, end to end.
      testInfo.setTimeout(profile().timeouts.sandboxReady * 3 + profile().timeouts.agentIdle * 5)

      const sse = new EventStream(driver)
      await sse.start()

      const workspace = await api.ensureTemplateWorkspace()
      const parent = await api.createVibeTask({
        workspaceId: workspace.id,
        name: `${harness.agentProfile} trajectory fork`,
        message: `Our first release is named ${FIRST_NAME}. Reply with exactly OK. Use no tools.`,
        agentProfile: harness.agentProfile,
        model: harness.model,
      })
      const created: string[] = [parent.taskId]

      try {
        const { ready } = await waitForSandboxReady(
          api,
          parent.taskId,
          profile().timeouts.sandboxReady,
        )
        expect(ready, 'the parent sandbox never became ready').toBe(true)
        await waitForTurns(sse, parent.taskId, 1)

        await api.postMessage(parent.taskId, {
          message: `Our second release is named ${SECOND_NAME}. Reply with exactly OK. Use no tools.`,
          model: harness.model,
          agentMode: 'code',
          startFresh: false,
        })
        await waitForTurns(sse, parent.taskId, 2)

        const fork = await api.forkTask(parent.taskId, { atTurn: 1, message: QUESTION })
        created.push(fork.taskId)
        const forkReady = await waitForSandboxReady(
          api,
          fork.taskId,
          profile().timeouts.sandboxReady,
        )
        expect(forkReady.ready, 'the fork sandbox never became ready').toBe(true)
        await waitForTurns(sse, fork.taskId, 1)

        const forkAnswer = await ownAnswers(api, fork.taskId)
        expect(forkAnswer, 'the fork lost the history before its cut').toContain(FIRST_NAME)
        expect(forkAnswer, 'the fork saw a Turn after its cut').not.toContain(SECOND_NAME)

        const stopped = await stopSandbox(api, parent.taskId)
        expect(stopped, 'the parent sandbox never stopped').not.toBeNull()
        await api.postMessage(parent.taskId, {
          message: QUESTION,
          model: harness.model,
          agentMode: 'code',
          startFresh: false,
        })
        const replaced = await waitForSandboxReady(
          api,
          parent.taskId,
          profile().timeouts.sandboxReady,
        )
        expect(replaced.ready, 'the replacement sandbox never became ready').toBe(true)
        await waitForTurns(sse, parent.taskId, 3)

        const recovered = await ownAnswers(api, parent.taskId)
        expect(recovered, 'the replaced pod lost the first Turn').toContain(FIRST_NAME)
        expect(recovered, 'the replaced pod lost the second Turn').toContain(SECOND_NAME)
      } finally {
        sse.stop()
        for (const taskId of created) {
          await api.stopTask(taskId).catch(() => {})
          await api.deleteTask(taskId).catch(() => {})
        }
      }
    })
  })
}
