/**
 * Standalone: Harness switching
 *
 * A Task switches its Agent profile between turns. A switch into the other
 * harness renders the history into that harness's session file; a switch back
 * forks the earlier session verbatim.
 *
 * Scenarios, as one chain over one parent Task:
 *   SWITCH-01: Claude Turn 1 writes release.txt naming PERIWINKLE
 *   SWITCH-02: Codex Turn 2 knows what Turn 1 wrote, and where
 *   SWITCH-04: Turn 1's Messages appear once after the switch
 *   SWITCH-05: A replaced pod resumes the Codex session from the Trajectory
 *   SWITCH-03: Back on Claude, a later Turn knows every name
 *   SWITCH-06: claude-code → claude-code-cli answers from history
 *   SWITCH-07: A fork at Turn 1 into Codex knows Turn 1 only
 *
 * SWITCH-08 (rewind into a Codex turn) has no REST rewind to drive it;
 * `crates/tasks/src/task/rewind_db_tests.rs` covers it.
 */

import { findNamedFileWithContent } from '../../shared/content-helpers'
import { EventStream } from '../../shared/event-stream'
import type { MoatApi } from '../../shared/moat-api'
import { missingCapability, profile } from '../../shared/profile'
import { expect, test } from '../fixtures/api-fixtures'
import { stopSandbox, waitForSandboxReady } from '../steps'

const FIRST_NAME = 'PERIWINKLE'
const SECOND_NAME = 'SAFFRON'
const FILE = 'release.txt'
const CLAUDE_MODEL = 'haiku'
const CODEX_MODEL = 'gpt-5.5'
const LIST = 'List every release name I have told you so far, separated by commas. Use no tools.'

/** The latest Turn's own assistant text and the model that wrote it. */
async function lastAnswer(api: MoatApi, taskId: string): Promise<{ text: string; model: string }> {
  const { messages } = await api.getTaskMessages(taskId)
  const own = messages.filter((message) => message.role === 'assistant' && !message.sourceTaskId)
  const lastTurn = Math.max(...own.map((message) => message.turnNumber ?? 0))
  const latest = own.filter((message) => (message.turnNumber ?? 0) === lastTurn)
  return {
    text: latest
      .flatMap((message) => message.content)
      .map((block) => (block.type === 'text' ? block.text : ''))
      .join('\n'),
    model: latest.find((message) => message.model)?.model ?? '',
  }
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

async function send(api: MoatApi, taskId: string, message: string, model: string): Promise<void> {
  await api.postMessage(taskId, { message, model, agentMode: 'code', startFresh: false })
}

test.describe('Standalone: Harness switching', () => {
  const reason = missingCapability('claudeCredentials') ?? missingCapability('codexConnection')
  test.skip(reason !== null, reason ?? '')

  test('SWITCH-01..07: A Task switches harness and back, and forks across one', async ({
    api,
    driver,
  }, testInfo) => {
    // Three provisions and seven Turns, end to end.
    testInfo.setTimeout(profile().timeouts.sandboxReady * 3 + profile().timeouts.agentIdle * 7)

    await api.storeHarnessCredentials()
    const sse = new EventStream(driver)
    await sse.start()

    const workspace = await api.ensureTemplateWorkspace()
    const parent = await api.createVibeTask({
      workspaceId: workspace.id,
      name: 'harness switch',
      message: `Our first release is named ${FIRST_NAME}. Write that name into a file ${FILE} at the root of the repository, then reply with exactly OK.`,
      agentProfile: 'claude-code',
      model: CLAUDE_MODEL,
    })
    const created: string[] = [parent.taskId]

    try {
      // SWITCH-01
      const { ready } = await waitForSandboxReady(
        api,
        parent.taskId,
        profile().timeouts.sandboxReady,
      )
      expect(ready, 'the sandbox never became ready').toBe(true)
      await waitForTurns(sse, parent.taskId, 1)
      expect(
        await findNamedFileWithContent(api, parent.taskId, FILE, FIRST_NAME),
        `Turn 1 wrote no ${FILE}`,
      ).not.toBeNull()

      // SWITCH-02
      const switched = await api.switchProfile(parent.taskId, 'codex', CODEX_MODEL)
      expect(switched.agentProfile).toBe('codex')
      await send(
        api,
        parent.taskId,
        'What release name did I tell you, and which file did you write it to? Use no tools.',
        CODEX_MODEL,
      )
      await waitForTurns(sse, parent.taskId, 2)
      const codex = await lastAnswer(api, parent.taskId)
      expect(codex.text, 'Codex lost the Claude turn').toContain(FIRST_NAME)
      expect(codex.text, 'Codex lost the tool call').toContain(FILE)
      expect(codex.model).toMatch(/^gpt-/)

      // SWITCH-04
      const { messages } = await api.getTaskMessages(parent.taskId)
      const firstPrompts = messages.filter(
        (message) =>
          message.role === 'user' &&
          message.content.some(
            (block) => block.type === 'text' && (block.text ?? '').includes(FIRST_NAME),
          ),
      )
      expect(firstPrompts, 'the transcoded prefix was streamed again').toHaveLength(1)

      // SWITCH-05
      expect(await stopSandbox(api, parent.taskId), 'the sandbox never stopped').not.toBeNull()
      await send(api, parent.taskId, LIST, CODEX_MODEL)
      const replaced = await waitForSandboxReady(
        api,
        parent.taskId,
        profile().timeouts.sandboxReady,
      )
      expect(replaced.ready, 'the replacement sandbox never became ready').toBe(true)
      await waitForTurns(sse, parent.taskId, 3)
      const recovered = await lastAnswer(api, parent.taskId)
      expect(recovered.text, 'the replaced pod lost the history').toContain(FIRST_NAME)
      expect(recovered.model).toMatch(/^gpt-/)

      // SWITCH-03
      await api.switchProfile(parent.taskId, 'claude-code', CLAUDE_MODEL)
      await send(
        api,
        parent.taskId,
        `Our second release is named ${SECOND_NAME}. Reply with exactly OK. Use no tools.`,
        CLAUDE_MODEL,
      )
      await waitForTurns(sse, parent.taskId, 4)
      await send(api, parent.taskId, LIST, CLAUDE_MODEL)
      await waitForTurns(sse, parent.taskId, 5)
      const back = await lastAnswer(api, parent.taskId)
      expect(back.text).toContain(FIRST_NAME)
      expect(back.text).toContain(SECOND_NAME)
      expect(back.model).toMatch(/^claude-/)

      // SWITCH-06
      await api.switchProfile(parent.taskId, 'claude-code-cli', CLAUDE_MODEL)
      await send(api, parent.taskId, LIST, CLAUDE_MODEL)
      await waitForTurns(sse, parent.taskId, 6)
      const cli = await lastAnswer(api, parent.taskId)
      expect(cli.text).toContain(FIRST_NAME)
      expect(cli.text).toContain(SECOND_NAME)

      // SWITCH-07
      const fork = await api.forkTask(parent.taskId, {
        atTurn: 1,
        message: LIST,
        agentProfile: 'codex',
      })
      created.push(fork.taskId)
      const forkReady = await waitForSandboxReady(api, fork.taskId, profile().timeouts.sandboxReady)
      expect(forkReady.ready, 'the fork sandbox never became ready').toBe(true)
      await waitForTurns(sse, fork.taskId, 1)
      const forked = await lastAnswer(api, fork.taskId)
      expect(forked.text, 'the fork lost the history before its cut').toContain(FIRST_NAME)
      expect(forked.text, 'the fork saw a Turn after its cut').not.toContain(SECOND_NAME)
      expect(forked.model).toMatch(/^gpt-/)
    } finally {
      sse.stop()
      for (const taskId of created) {
        await api.stopTask(taskId).catch(() => {})
        await api.deleteTask(taskId).catch(() => {})
      }
    }
  })
})
