/** Real mini-swe-agent execution, follow-up, and transcript recovery after sandbox replacement. */
import { readdir, rm } from 'node:fs/promises'
import { dirname, join } from 'node:path'
import { findNamedFileWithContent } from '../../shared/content-helpers'
import { EventStream } from '../../shared/event-stream'
import { missingCapability, profile } from '../../shared/profile'
import { scopedName } from '../../shared/run-scope'
import { expect, test } from '../fixtures/api-fixtures'
import { progress } from '../helpers/progress'
import { discardTask } from '../helpers/teardown'
import { stopSandbox, waitForSandboxReady } from '../steps'

test.describe('mini-swe-agent real-agent execution', () => {
  const reason = missingCapability('miniSweCredentials')
  test.skip(reason !== null, reason ?? '')

  for (const provider of ['anthropic', 'openai'] as const) {
    test(`MINI-${provider}: tools, forks, recovery, steering, and cancellation`, async ({
      api,
      driver,
    }, info) => {
      const key = provider === 'anthropic' ? 'ANTHROPIC_API_KEY' : 'OPENAI_API_KEY'
      test.skip(
        !process.env[key] && process.env.MINI_SWE_E2E_HUB !== '1',
        `${key} is required for real mini-swe-agent ${provider} execution`,
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
          name: scopedName(`mini-${provider}`),
          agentProfile: 'mini-swe',
          model,
          message: `Remember ${secret} in this conversation only; do not write it to any file. Create mini-marker.txt containing exactly FIRST. Reply OK.`,
        })
        taskId = task.taskId
        const ready = await waitForSandboxReady(api, taskId)
        expect(ready.ready, 'mini-swe-agent sandbox did not become ready').toBe(true)
        const first = await events.waitForNthEvent(
          'taskTurn',
          'completed',
          taskId,
          1,
          profile().timeouts.agentIdle,
        )
        expect(first, 'mini-swe-agent did not complete the file-creation turn').not.toBeNull()
        expect(
          await findNamedFileWithContent(api, taskId, 'mini-marker.txt', 'FIRST'),
          'mini-swe-agent did not execute the requested file write',
        ).not.toBeNull()
        progress(`${provider}: first real mini-swe-agent turn completed`)

        await api.postMessage(taskId, {
          message: `Remember ${later} in the conversation only. Change mini-marker.txt to contain exactly SECOND.`,
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
        expect(second, 'mini-swe-agent did not complete the follow-up turn').not.toBeNull()
        expect(
          await findNamedFileWithContent(api, taskId, 'mini-marker.txt', 'SECOND'),
          'mini-swe-agent follow-up did not change the file',
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
          'mini-swe-agent fork did not complete',
        ).not.toBeNull()
        const forkMessages = await api.getTaskMessages(forkId)
        const forkText = forkMessages.messages
          .filter((m) => m.role === 'assistant' && !m.sourceTaskId)
          .flatMap((m) => m.content)
          .map((b) => (b.type === 'text' ? b.text : ''))
          .join('\n')
        expect(forkText, 'mini-swe-agent fork lost history before its cut').toContain(secret)
        expect(forkText, 'mini-swe-agent fork leaked history after its cut').not.toContain(later)

        expect(await stopSandbox(api, taskId), 'mini-swe-agent sandbox did not stop').not.toBeNull()
        const localRoot = process.env.E2E_LOCAL_SANDBOX_ROOT
        if (localRoot) {
          const sessions = join(localRoot, 'tasks', taskId, 'home', '.mini-swe', 'sessions')
          expect(
            (await readdir(sessions)).length,
            'The local mini-swe-agent session was never persisted',
          ).toBeGreaterThan(0)
          await rm(sessions, { recursive: true })
        }
        await api.postMessage(taskId, {
          message: 'Without reading files, reply with only the MEMORY value from my first message.',
          model,
          startFresh: false,
        })
        const replacement = await waitForSandboxReady(api, taskId)
        expect(replacement.ready, 'mini-swe-agent replacement sandbox did not become ready').toBe(
          true,
        )
        const third = await events.waitForNthEvent(
          'taskTurn',
          'completed',
          taskId,
          3,
          profile().timeouts.agentIdle,
        )
        expect(third, 'mini-swe-agent did not complete after sandbox replacement').not.toBeNull()
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
          'mini-swe-agent lost conversation-only context after sandbox replacement',
        ).toContain(secret)
        expect(
          replies.some((message) => message.model?.includes(model.split('/')[1]!)),
          'mini-swe-agent reported the wrong model after recovery',
        ).toBe(true)
        await api.postMessage(taskId, {
          message:
            'Run bash to write mini-steering-ready.txt containing READY, then sleep 15 seconds. After the command, follow any queued instruction.',
          model,
        })
        await expect
          .poll(() => findNamedFileWithContent(api, taskId!, 'mini-steering-ready.txt', 'READY'), {
            timeout: profile().timeouts.agentIdle,
            message: 'mini-swe-agent did not enter the steering window',
          })
          .not.toBeNull()
        await api.postMessage(taskId, {
          message: 'Write mini-steered.txt containing STEERED, then reply DONE.',
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
          'mini-swe-agent did not settle the steered turn',
        ).not.toBeNull()
        expect(
          await findNamedFileWithContent(api, taskId, 'mini-steered.txt', 'STEERED'),
          'mini-swe-agent did not consume the live steering instruction',
        ).not.toBeNull()
        expect(
          events.countEvents('taskTurn', 'active', taskId),
          'Steering unexpectedly opened a separate turn',
        ).toBe(4)

        const toolDelaySeconds = Math.ceil(profile().timeouts.sandboxTeardown / 1000)
        await api.postMessage(taskId, {
          message: `Run bash to write mini-cancel-ready.txt containing READY, then sleep ${toolDelaySeconds} seconds. After sleep, write mini-cancel-failed.txt containing BAD.`,
          model,
        })
        await expect
          .poll(() => findNamedFileWithContent(api, taskId!, 'mini-cancel-ready.txt', 'READY'), {
            timeout: profile().timeouts.agentIdle,
            message: 'mini-swe-agent did not enter the cancellation window',
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
          'mini-swe-agent did not report the cancelled turn',
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
          'mini-swe-agent could not resume after cancellation',
        ).not.toBeNull()
        const resumed = await api.getTaskMessages(taskId)
        const resumedText = resumed.messages
          .filter((m) => m.role === 'assistant' && m.turnNumber === 6)
          .flatMap((m) => m.content)
          .map((b) => (b.type === 'text' ? b.text : ''))
          .join('\n')
        expect(
          resumedText,
          'mini-swe-agent lost conversation history after cancellation',
        ).toContain(secret)
        await new Promise((resolve) => setTimeout(resolve, (toolDelaySeconds + 1) * 1000))
        expect(
          await findNamedFileWithContent(api, taskId, 'mini-cancel-failed.txt', 'BAD'),
          'The cancelled mini-swe-agent tool continued running',
        ).toBeNull()
      } finally {
        await events.stop()
        await discardTask(api, forkId)
        await discardTask(api, taskId)
      }
    })
  }
})

test('MINI-switch: Claude history survives a switch into mini-swe-agent and back', async ({
  api,
  driver,
}, info) => {
  test.skip(
    !process.env.ANTHROPIC_API_KEY && process.env.MINI_SWE_E2E_HUB !== '1',
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
      name: scopedName('mini-switch'),
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
    await api.switchProfile(taskId, 'mini-swe', 'anthropic/claude-haiku-5-5')
    await api.postMessage(taskId, {
      message: `Remember ${second} only in this conversation. Without reading files, reply with the FIRST value I told you.`,
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
      'mini-swe-agent did not continue the Claude conversation',
    ).not.toBeNull()
    const pi = await api.getTaskMessages(taskId)
    const text = pi.messages
      .filter((m) => m.role === 'assistant' && m.turnNumber === 2)
      .flatMap((m) => m.content)
      .map((b) => (b.type === 'text' ? b.text : ''))
      .join('\n')
    expect(text, 'mini-swe-agent lost the source Claude conversation').toContain(first)
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
      'Claude did not continue the mini-swe-agent conversation',
    ).not.toBeNull()
    const back = await api.getTaskMessages(taskId)
    const answer = back.messages
      .filter((m) => m.role === 'assistant' && m.turnNumber === 3)
      .flatMap((m) => m.content)
      .map((b) => (b.type === 'text' ? b.text : ''))
      .join('\n')
    expect(answer, 'Switching back lost the Claude history').toContain(first)
    expect(answer, 'Switching back lost the mini-swe-agent history').toContain(second)
  } finally {
    await events.stop()
    await discardTask(api, taskId)
  }
})

test('MINI-provider-switch: native history survives OpenAI to Anthropic and back', async ({
  api,
  driver,
}, info) => {
  test.skip(
    process.env.MINI_SWE_E2E_HUB !== '1' &&
      !(process.env.OPENAI_API_KEY && process.env.ANTHROPIC_API_KEY),
    'Both provider credentials or the test hub are required',
  )
  info.setTimeout(
    profile().timeouts.sandboxReady +
      profile().timeouts.agentIdle * 3 +
      profile().timeouts.sandboxTeardown,
  )
  await api.storeHarnessCredentials()
  const workspace = await api.ensureTemplateWorkspace()
  const events = new EventStream(driver)
  await events.start()
  let taskId: string | undefined
  const value = `PROVIDER-${crypto.randomUUID()}`
  try {
    const task = await api.createVibeTask({
      workspaceId: workspace.id,
      name: scopedName('mini-provider-switch'),
      agentProfile: 'mini-swe',
      model: 'openai/gpt-6.1-sol',
      message: `Remember ${value} only in conversation. Use bash to print READY, then finish.`,
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
      'OpenAI did not establish native tool history',
    ).not.toBeNull()
    for (const [index, model] of ['anthropic/claude-sonnet-5-5', 'openai/gpt-6.1-sol'].entries()) {
      await api.postMessage(taskId, {
        message: 'Without reading files, reply with the PROVIDER value I told you.',
        model,
      })
      expect(
        await events.waitForNthEvent(
          'taskTurn',
          'completed',
          taskId,
          index + 2,
          profile().timeouts.agentIdle,
        ),
        `Native history could not continue on ${model}`,
      ).not.toBeNull()
      const reply = await api.getTaskMessages(taskId)
      const text = reply.messages
        .filter((m) => m.role === 'assistant' && m.turnNumber === index + 2)
        .flatMap((m) => m.content)
        .map((b) => (b.type === 'text' ? b.text : ''))
        .join('\n')
      expect(text, `Changing provider to ${model} lost conversation history`).toContain(value)
    }
  } finally {
    await events.stop()
    await discardTask(api, taskId)
  }
})

test('MINI-skills: discovers a skill and follows its relative resource', async ({
  api,
  driver,
}, info) => {
  const reason = missingCapability('miniSweCredentials')
  test.skip(reason !== null, reason ?? '')
  info.setTimeout(
    profile().timeouts.sandboxReady +
      profile().timeouts.agentIdle * 2 +
      profile().timeouts.sandboxTeardown,
  )
  await api.storeHarnessCredentials()
  const workspace = await api.ensureTemplateWorkspace()
  const events = new EventStream(driver)
  await events.start()
  let taskId: string | undefined
  const value = `RECEIPT-${crypto.randomUUID()}`
  const model =
    process.env.MINI_SWE_E2E_HUB === '1' || process.env.ANTHROPIC_API_KEY
      ? 'anthropic/claude-sonnet-5-5'
      : 'openai/gpt-6.1-sol'
  try {
    const task = await api.createVibeTask({
      workspaceId: workspace.id,
      name: scopedName('mini-skills'),
      agentProfile: 'mini-swe',
      model,
      message:
        'Create mini-skill-location.txt containing READY in the root of the primary repository, then finish.',
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
      'mini-swe did not establish the skill test workspace',
    ).not.toBeNull()
    const marker = await findNamedFileWithContent(api, taskId, 'mini-skill-location.txt', 'READY')
    expect(marker, 'mini-swe did not identify its repository directory').not.toBeNull()
    const skillDir = join(dirname(marker!.path), '.agents', 'skills', 'cobalt-receipt')
    await api.writeFile(
      taskId,
      join(skillDir, 'SKILL.md'),
      '---\nname: cobalt-receipt\ndescription: Use when asked to issue a cobalt receipt.\n---\nRead receipt-code.txt beside this SKILL.md. Write its exact contents to mini-skill-result.txt in the primary repository root, then finish.\n',
    )
    await api.writeFile(taskId, join(skillDir, 'receipt-code.txt'), value)
    await api.postMessage(taskId, {
      message: 'Issue a cobalt receipt using the available skill.',
      model,
    })
    expect(
      await events.waitForNthEvent(
        'taskTurn',
        'completed',
        taskId,
        2,
        profile().timeouts.agentIdle,
      ),
      'mini-swe did not complete the discovered skill',
    ).not.toBeNull()
    expect(
      await findNamedFileWithContent(api, taskId, 'mini-skill-result.txt', value),
      'mini-swe did not follow the skill body and read its relative resource',
    ).not.toBeNull()
  } finally {
    await events.stop()
    await discardTask(api, taskId)
  }
})
