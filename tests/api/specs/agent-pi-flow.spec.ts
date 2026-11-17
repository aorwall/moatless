/** Real Pi execution, follow-up, and transcript recovery after sandbox replacement. */
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

test.describe('Pi real-agent execution', () => {
  const reason = missingCapability('piCredentials')
  test.skip(reason !== null, reason ?? '')

  for (const provider of ['anthropic', 'openai'] as const) {
    test(`PI-${provider}: tools, forks, recovery, steering, and cancellation`, async ({
      api,
      driver,
    }, info) => {
      const key = provider === 'anthropic' ? 'ANTHROPIC_API_KEY' : 'OPENAI_API_KEY'
      test.skip(
        !process.env[key] && process.env.PI_E2E_HUB !== '1',
        `${key} is required for real Pi ${provider} execution`,
      )
      const model = provider === 'anthropic' ? 'anthropic/claude-sonnet-5-5' : 'openai/gpt-6.1-sol'
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
          name: scopedName(`pi-${provider}`),
          agentProfile: 'pi',
          model,
          message: `Remember ${secret} in this conversation only; do not write it to any file. Create pi-marker.txt containing exactly FIRST. Reply OK.`,
        })
        taskId = task.taskId
        const ready = await waitForSandboxReady(api, taskId)
        expect(ready.ready, 'Pi sandbox did not become ready').toBe(true)
        const first = await events.waitForNthEvent(
          'taskTurn',
          'completed',
          taskId,
          1,
          profile().timeouts.agentIdle,
        )
        expect(first, 'Pi did not complete the file-creation turn').not.toBeNull()
        expect(
          await findNamedFileWithContent(api, taskId, 'pi-marker.txt', 'FIRST'),
          'Pi did not execute the requested file write',
        ).not.toBeNull()
        progress(`${provider}: first real Pi turn completed`)

        await api.postMessage(taskId, {
          message: `Remember ${later} in the conversation only. Change pi-marker.txt to contain exactly SECOND.`,
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
        expect(second, 'Pi did not complete the follow-up turn').not.toBeNull()
        expect(
          await findNamedFileWithContent(api, taskId, 'pi-marker.txt', 'SECOND'),
          'Pi follow-up did not change the file',
        ).not.toBeNull()

        const fork = await api.forkTask(taskId, {
          atTurn: 1,
          message: 'Without tools, list every MEMORY and LATER value I have told you.',
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
          'Pi fork did not complete',
        ).not.toBeNull()
        const forkMessages = await api.getTaskMessages(forkId)
        const forkText = forkMessages.messages
          .filter((m) => m.role === 'assistant' && !m.sourceTaskId)
          .flatMap((m) => m.content)
          .map((b) => (b.type === 'text' ? b.text : ''))
          .join('\n')
        expect(forkText, 'Pi fork lost history before its cut').toContain(secret)
        expect(forkText, 'Pi fork leaked history after its cut').not.toContain(later)

        expect(await stopSandbox(api, taskId), 'Pi sandbox did not stop').not.toBeNull()
        const localRoot = process.env.E2E_LOCAL_SANDBOX_ROOT
        if (localRoot) {
          const sessions = join(localRoot, 'tasks', taskId, 'home', '.pi', 'agent', 'sessions')
          expect(
            (await readdir(sessions)).length,
            'The local Pi session was never persisted',
          ).toBeGreaterThan(0)
          await rm(sessions, { recursive: true })
        }
        await api.postMessage(taskId, {
          message: 'Without using tools, reply with only the MEMORY value from my first message.',
          model,
          startFresh: false,
        })
        const replacement = await waitForSandboxReady(api, taskId)
        expect(replacement.ready, 'Pi replacement sandbox did not become ready').toBe(true)
        const third = await events.waitForNthEvent(
          'taskTurn',
          'completed',
          taskId,
          3,
          profile().timeouts.agentIdle,
        )
        expect(third, 'Pi did not complete after sandbox replacement').not.toBeNull()
        const { messages } = await api.getTaskMessages(taskId)
        const replies = messages.filter(
          (message) => message.role === 'assistant' && message.turnNumber === 3,
        )
        const text = replies
          .flatMap((message) => message.content)
          .map((block) => (block.type === 'text' ? block.text : ''))
          .join('\n')
        expect(text, 'Pi lost conversation-only context after sandbox replacement').toContain(
          secret,
        )
        expect(
          replies.some((message) => message.model?.includes(model.split('/')[1]!)),
          'Pi reported the wrong model after recovery',
        ).toBe(true)
        await api.postMessage(taskId, {
          message:
            'Run bash to write pi-steering-ready.txt containing READY, then sleep 15 seconds. After the command, follow any queued instruction.',
          model,
        })
        await expect
          .poll(() => findNamedFileWithContent(api, taskId!, 'pi-steering-ready.txt', 'READY'), {
            timeout: profile().timeouts.agentIdle,
            message: 'Pi did not enter the steering window',
          })
          .not.toBeNull()
        await api.postMessage(taskId, {
          message: 'Write pi-steered.txt containing STEERED, then reply DONE.',
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
          'Pi did not settle the steered turn',
        ).not.toBeNull()
        expect(
          await findNamedFileWithContent(api, taskId, 'pi-steered.txt', 'STEERED'),
          'Pi did not consume the live steering instruction',
        ).not.toBeNull()
        expect(
          events.countEvents('taskTurn', 'active', taskId),
          'Steering unexpectedly opened a separate turn',
        ).toBe(4)

        await api.postMessage(taskId, {
          message:
            'Run bash to write pi-cancel-ready.txt containing READY, then sleep 60 seconds. After sleep, write pi-cancel-failed.txt containing BAD.',
          model,
        })
        await expect
          .poll(() => findNamedFileWithContent(api, taskId!, 'pi-cancel-ready.txt', 'READY'), {
            timeout: profile().timeouts.agentIdle,
            message: 'Pi did not enter the cancellation window',
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
          'Pi did not report the cancelled turn',
        ).not.toBeNull()
        await api.postMessage(taskId, {
          message: 'Without tools, reply with only the MEMORY value from my first message.',
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
          'Pi could not resume after cancellation',
        ).not.toBeNull()
        const resumed = await api.getTaskMessages(taskId)
        const resumedText = resumed.messages
          .filter((m) => m.role === 'assistant' && m.turnNumber === 6)
          .flatMap((m) => m.content)
          .map((b) => (b.type === 'text' ? b.text : ''))
          .join('\n')
        expect(resumedText, 'Pi lost conversation history after cancellation').toContain(secret)
        expect(
          await findNamedFileWithContent(api, taskId, 'pi-cancel-failed.txt', 'BAD'),
          'The cancelled Pi tool continued running',
        ).toBeNull()
      } finally {
        await events.stop()
        await discardTask(api, forkId)
        await discardTask(api, taskId)
      }
    })
  }
})

test('PI-switch: Claude history survives a switch into Pi and back', async ({
  api,
  driver,
}, info) => {
  test.skip(
    !process.env.ANTHROPIC_API_KEY && process.env.PI_E2E_HUB !== '1',
    'An Anthropic API key or the test hub is required',
  )
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
      name: scopedName('pi-switch'),
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
    await api.switchProfile(taskId, 'pi', 'anthropic/claude-haiku-5-5')
    await api.postMessage(taskId, {
      message: `Remember ${second} only in this conversation. Without tools, reply with the FIRST value I told you.`,
      model: 'anthropic/claude-haiku-5-5',
    })
    expect(
      await events.waitForNthEvent(
        'taskTurn',
        'completed',
        taskId,
        2,
        profile().timeouts.agentIdle,
      ),
      'Pi did not continue the Claude conversation',
    ).not.toBeNull()
    const pi = await api.getTaskMessages(taskId)
    const text = pi.messages
      .filter((m) => m.role === 'assistant' && m.turnNumber === 2)
      .flatMap((m) => m.content)
      .map((b) => (b.type === 'text' ? b.text : ''))
      .join('\n')
    expect(text, 'Pi lost the source Claude conversation').toContain(first)
    await api.switchProfile(taskId, 'claude-code-cli', 'haiku')
    await api.postMessage(taskId, {
      message: 'Without tools, list the FIRST and SECOND values I told you.',
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
      'Claude did not continue the Pi conversation',
    ).not.toBeNull()
    const back = await api.getTaskMessages(taskId)
    const answer = back.messages
      .filter((m) => m.role === 'assistant' && m.turnNumber === 3)
      .flatMap((m) => m.content)
      .map((b) => (b.type === 'text' ? b.text : ''))
      .join('\n')
    expect(answer, 'Switching back lost the Claude history').toContain(first)
    expect(answer, 'Switching back lost the Pi history').toContain(second)
  } finally {
    await events.stop()
    await discardTask(api, taskId)
  }
})
