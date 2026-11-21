/** Real Antigravity execution over the ACP harness: tools, forks, recovery, steering and cancellation. */
import { readdir, rm } from 'node:fs/promises'
import { join } from 'node:path'
import { findNamedFileWithContent } from '../../shared/content-helpers'
import { EventStream } from '../../shared/event-stream'
import { missingCapability, profile } from '../../shared/profile'
import { scopedName } from '../../shared/run-scope'
import { expect, test } from '../fixtures/api-fixtures'
import { progress } from '../helpers/progress'
import { discardTask } from '../helpers/teardown'
import { stopSandbox, waitForSandboxReady } from '../steps'

test.describe('Antigravity real-agent execution', () => {
  const reason = missingCapability('antigravityCredentials')
  test.skip(reason !== null, reason ?? '')

  for (const model of ['gemini-3-flash'] as const) {
    test(`AGY-${model}: tools, forks, recovery, steering, and cancellation`, async ({
      api,
      driver,
    }, info) => {
      const provider = model
      info.setTimeout(
        profile().timeouts.sandboxReady * 3 +
          profile().timeouts.agentIdle * 7 +
          profile().timeouts.sandboxTeardown,
      )
      await api.storeHarnessCredentials()
      const workspace = await api.ensureTemplateWorkspace()
      const events = new EventStream(driver)
      await events.start()
      let taskId: string | undefined
      let forkId: string | undefined
      const secret = `MEMORY-${crypto.randomUUID()}`
      const later = `LATER-${crypto.randomUUID()}`
      try {
        const task = await api.createVibeTask({
          workspaceId: workspace.id,
          name: scopedName(`agy-${provider}`),
          agentProfile: 'antigravity',
          model,
          message: `Remember ${secret} in this conversation only; do not write it to any file. Create agy-marker.txt containing exactly FIRST. Reply OK.`,
        })
        taskId = task.taskId
        const ready = await waitForSandboxReady(api, taskId)
        expect(ready.ready, 'Antigravity sandbox did not become ready').toBe(true)
        const first = await events.waitForNthEvent(
          'taskTurn',
          'completed',
          taskId,
          1,
          profile().timeouts.agentIdle,
        )
        expect(first, 'Antigravity did not complete the file-creation turn').not.toBeNull()
        expect(
          await findNamedFileWithContent(api, taskId, 'agy-marker.txt', 'FIRST'),
          'Antigravity did not execute the requested file write',
        ).not.toBeNull()
        progress(`${provider}: first real Antigravity turn completed`)

        await api.postMessage(taskId, {
          message: `Remember ${later} in the conversation only. Change agy-marker.txt to contain exactly SECOND.`,
          model,
          startFresh: false,
        })
        const second = await events.waitForNthEvent(
          'taskTurn',
          'completed',
          taskId,
          2,
          profile().timeouts.agentIdle,
        )
        expect(second, 'Antigravity did not complete the follow-up turn').not.toBeNull()
        expect(
          await findNamedFileWithContent(api, taskId, 'agy-marker.txt', 'SECOND'),
          'Antigravity follow-up did not change the file',
        ).not.toBeNull()

        const fork = await api.forkTask(taskId, {
          atTurn: 1,
          message: 'Without reading files, list every MEMORY and LATER value I have told you.',
        })
        forkId = fork.taskId
        expect(
          await events.waitForNthEvent(
            'taskTurn',
            'completed',
            forkId,
            1,
            profile().timeouts.agentIdle,
          ),
          'Antigravity fork did not complete',
        ).not.toBeNull()
        const forkMessages = await api.getTaskMessages(forkId)
        const forkText = forkMessages.messages
          .filter((m) => m.role === 'assistant' && !m.sourceTaskId)
          .flatMap((m) => m.content)
          .map((b) => (b.type === 'text' ? b.text : ''))
          .join('\n')
        expect(forkText, 'Antigravity fork lost history before its cut').toContain(secret)
        expect(forkText, 'Antigravity fork leaked history after its cut').not.toContain(later)

        expect(await stopSandbox(api, taskId), 'Antigravity sandbox did not stop').not.toBeNull()
        const localRoot = process.env.E2E_LOCAL_SANDBOX_ROOT
        if (localRoot) {
          // The journal and agy's own conversation both come back from the archive.
          const home = join(localRoot, 'tasks', taskId, 'home', '.acp')
          expect(
            (await readdir(join(home, 'sessions'))).length,
            'The local ACP journal was never persisted',
          ).toBeGreaterThan(0)
          await rm(home, { recursive: true })
        }
        await api.postMessage(taskId, {
          message: 'Without reading files, reply with only the MEMORY value from my first message.',
          model,
          startFresh: false,
        })
        const replacement = await waitForSandboxReady(api, taskId)
        expect(replacement.ready, 'Antigravity replacement sandbox did not become ready').toBe(true)
        const third = await events.waitForNthEvent(
          'taskTurn',
          'completed',
          taskId,
          3,
          profile().timeouts.agentIdle,
        )
        expect(third, 'Antigravity did not complete after sandbox replacement').not.toBeNull()
        const { messages } = await api.getTaskMessages(taskId)
        const replies = messages.filter(
          (message) => message.role === 'assistant' && message.turnNumber === 3,
        )
        const text = replies
          .flatMap((message) => message.content)
          .map((block) => (block.type === 'text' ? block.text : ''))
          .join('\n')
        expect(
          text,
          'Antigravity lost conversation-only context after sandbox replacement',
        ).toContain(secret)
        expect(
          replies.some((message) => message.model?.includes(model)),
          'Antigravity reported the wrong model after recovery',
        ).toBe(true)
        await api.postMessage(taskId, {
          message:
            'Run a shell command to write agy-steering-ready.txt containing READY, then sleep 15 seconds. After the command, follow any queued instruction.',
          model,
        })
        await expect
          .poll(() => findNamedFileWithContent(api, taskId!, 'agy-steering-ready.txt', 'READY'), {
            timeout: profile().timeouts.agentIdle,
            message: 'Antigravity did not enter the steering window',
          })
          .not.toBeNull()
        await api.postMessage(taskId, {
          message: 'Write agy-steered.txt containing STEERED, then reply DONE.',
          model,
        })
        expect(
          await events.waitForNthEvent(
            'taskTurn',
            'completed',
            taskId,
            4,
            profile().timeouts.agentIdle,
          ),
          'Antigravity did not settle the steered turn',
        ).not.toBeNull()
        expect(
          await findNamedFileWithContent(api, taskId, 'agy-steered.txt', 'STEERED'),
          'Antigravity did not consume the live steering instruction',
        ).not.toBeNull()
        expect(
          events.countEvents('taskTurn', 'active', taskId),
          'Steering unexpectedly opened a separate turn',
        ).toBe(4)

        const toolDelaySeconds = Math.ceil(profile().timeouts.sandboxTeardown / 1000)
        await api.postMessage(taskId, {
          message: `Run a shell command to write agy-cancel-ready.txt containing READY, then sleep ${toolDelaySeconds} seconds. After sleep, write agy-cancel-failed.txt containing BAD.`,
          model,
        })
        await expect
          .poll(() => findNamedFileWithContent(api, taskId!, 'agy-cancel-ready.txt', 'READY'), {
            timeout: profile().timeouts.agentIdle,
            message: 'Antigravity did not enter the cancellation window',
          })
          .not.toBeNull()
        await api.stopTask(taskId)
        expect(
          await events.waitForEvent({
            type: 'taskTurn',
            subType: 'stopped',
            taskId,
            timeout: profile().timeouts.agentIdle,
          }),
          'Antigravity did not report the cancelled turn',
        ).not.toBeNull()
        await api.postMessage(taskId, {
          message: 'Without reading files, reply with only the MEMORY value from my first message.',
          model,
        })
        expect(
          await events.waitForNthEvent(
            'taskTurn',
            'completed',
            taskId,
            5,
            profile().timeouts.agentIdle,
          ),
          'Antigravity could not resume after cancellation',
        ).not.toBeNull()
        const resumed = await api.getTaskMessages(taskId)
        const resumedText = resumed.messages
          .filter((m) => m.role === 'assistant' && m.turnNumber === 6)
          .flatMap((m) => m.content)
          .map((b) => (b.type === 'text' ? b.text : ''))
          .join('\n')
        expect(resumedText, 'Antigravity lost conversation history after cancellation').toContain(
          secret,
        )
        await new Promise((resolve) => setTimeout(resolve, (toolDelaySeconds + 1) * 1000))
        expect(
          await findNamedFileWithContent(api, taskId, 'agy-cancel-failed.txt', 'BAD'),
          'The cancelled Antigravity tool continued running',
        ).toBeNull()
      } finally {
        await events.stop()
        await discardTask(api, forkId)
        await discardTask(api, taskId)
      }
    })
  }
})

test('AGY-switch: Claude history survives a switch into Antigravity and back', async ({
  api,
  driver,
}, info) => {
  const reason = missingCapability('antigravityCredentials')
  test.skip(reason !== null, reason ?? '')
  info.setTimeout(profile().timeouts.sandboxReady + profile().timeouts.agentIdle * 3)
  await api.storeHarnessCredentials()
  const workspace = await api.ensureTemplateWorkspace()
  const events = new EventStream(driver)
  await events.start()
  let taskId: string | undefined
  const first = `FIRST-${crypto.randomUUID()}`
  const second = `SECOND-${crypto.randomUUID()}`
  try {
    const task = await api.createVibeTask({
      workspaceId: workspace.id,
      name: scopedName('agy-switch'),
      agentProfile: 'claude-code-cli',
      model: 'haiku',
      message: `Remember ${first} only in this conversation. Use no tools. Reply OK.`,
    })
    taskId = task.taskId
    expect(
      await events.waitForNthEvent(
        'taskTurn',
        'completed',
        taskId,
        1,
        profile().timeouts.agentIdle,
      ),
      'Claude did not establish the source history',
    ).not.toBeNull()
    await api.switchProfile(taskId, 'antigravity', 'gemini-3-flash')
    await api.postMessage(taskId, {
      message: `Remember ${second} only in this conversation. Without reading files, reply with the FIRST value I told you.`,
      model: 'gemini-3-flash',
    })
    expect(
      await events.waitForNthEvent(
        'taskTurn',
        'completed',
        taskId,
        2,
        profile().timeouts.agentIdle,
      ),
      'Antigravity did not continue the Claude conversation',
    ).not.toBeNull()
    const agy = await api.getTaskMessages(taskId)
    const text = agy.messages
      .filter((m) => m.role === 'assistant' && m.turnNumber === 2)
      .flatMap((m) => m.content)
      .map((b) => (b.type === 'text' ? b.text : ''))
      .join('\n')
    expect(text, 'Antigravity lost the source Claude conversation').toContain(first)
    await api.switchProfile(taskId, 'claude-code-cli', 'haiku')
    await api.postMessage(taskId, {
      message: 'Without reading files, list the FIRST and SECOND values I told you.',
      model: 'haiku',
    })
    expect(
      await events.waitForNthEvent(
        'taskTurn',
        'completed',
        taskId,
        3,
        profile().timeouts.agentIdle,
      ),
      'Claude did not continue the Antigravity conversation',
    ).not.toBeNull()
    const back = await api.getTaskMessages(taskId)
    const answer = back.messages
      .filter((m) => m.role === 'assistant' && m.turnNumber === 3)
      .flatMap((m) => m.content)
      .map((b) => (b.type === 'text' ? b.text : ''))
      .join('\n')
    expect(answer, 'Switching back lost the Claude history').toContain(first)
    expect(answer, 'Switching back lost the Antigravity history').toContain(second)
  } finally {
    await events.stop()
    await discardTask(api, taskId)
  }
})
