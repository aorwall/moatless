/**
 * Standalone: Agent tool interaction (specs/agent/01-agent-scenarios.md)
 *
 * Two things the agent does with tools that only show up end to end: pausing a
 * turn to ask the user a question, and spawning a subagent whose messages have
 * to be attributable to the tool call that spawned them.
 *
 * Both drive a real agent, so both need agent credentials.
 *
 * Scenarios:
 *   AGENT-07: Answering the agent's question resumes the same turn
 *   AGENT-08: Subagent messages name the tool call that spawned them
 */

import { EventStream } from '../../shared/event-stream'
import { missingCapability, profile } from '../../shared/profile'
import { expect, test } from '../fixtures/api-fixtures'
import { waitForPendingToolCall, waitForSandboxReady } from '../steps'

const reason = missingCapability('agentCredentials')

// ─── AGENT-07: Answering the agent's question resumes the same turn ────

test.describe('Standalone: Agent asks the user a question', () => {
  test.skip(reason !== null, reason ?? '')

  test("AGENT-07: Answering the agent's question resumes the same turn", async ({
    api,
    driver,
  }, testInfo) => {
    testInfo.setTimeout(profile().timeouts.sandboxReady + profile().timeouts.agentIdle * 2)

    const sse = new EventStream(driver)
    await sse.start()

    const repo = await api.ensureTemplateRepository()
    const task = await api.createVibeTask({
      templateId: repo.id,
      name: 'Agent AskUserQuestion',
      message:
        'Before making any changes, use the AskUserQuestion tool to ask me ' +
        'what my favorite color is. Then create a file called favorite-color.txt ' +
        'with whatever color I tell you.',
    })

    try {
      const { ready } = await waitForSandboxReady(api, task.taskId, profile().timeouts.sandboxReady)
      expect(ready).toBe(true)

      // Whether the agent asks is the model's decision, so wait for either
      // outcome and only assert on the one that happened.
      const outcome = await waitForPendingToolCall(
        sse,
        api,
        task.taskId,
        'AskUserQuestion',
        profile().timeouts.agentIdle,
      )
      expect(
        outcome.toolUseId !== null || outcome.completed,
        'the agent neither asked nor finished',
      ).toBe(true)

      if (outcome.toolUseId) {
        await api.sendToolResponse(task.taskId, {
          toolName: 'AskUserQuestion',
          toolUseId: outcome.toolUseId,
          toolResponse: { answers: { text: 'Blue' } },
        })

        // The answer has to reach the paused session — a turn that never
        // completes is the regression this covers.
        const completed = await sse.waitForEvent({
          type: 'taskTurn',
          subType: 'completed',
          taskId: task.taskId,
          timeout: profile().timeouts.agentIdle,
        })
        expect(completed, 'the answered turn never completed').not.toBeNull()
      }

      const { messages } = await api.getTaskMessages(task.taskId)
      expect(messages.length).toBeGreaterThanOrEqual(2)
    } finally {
      sse.stop()
      await api.stopTask(task.taskId).catch(() => {})
      await api.deleteTask(task.taskId).catch(() => {})
    }
  })
})

// ─── AGENT-08: Subagent messages name the tool call that spawned them ──

test.describe('Standalone: Subagent message attribution', () => {
  test.skip(reason !== null, reason ?? '')

  test('AGENT-08: Subagent messages name the tool call that spawned them', async ({
    api,
    driver,
  }, testInfo) => {
    testInfo.setTimeout(profile().timeouts.sandboxReady + profile().timeouts.agentIdle)

    const sse = new EventStream(driver)
    await sse.start()

    const repo = await api.ensureTemplateRepository()
    const task = await api.createVibeTask({
      templateId: repo.id,
      name: 'Subagent tracking',
      message:
        'Use the Agent tool to list the files in the current directory and ' +
        'tell me what you found.',
    })

    try {
      const { ready } = await waitForSandboxReady(api, task.taskId, profile().timeouts.sandboxReady)
      expect(ready).toBe(true)

      const completed = await sse.waitForEvent({
        type: 'taskTurn',
        subType: 'completed',
        taskId: task.taskId,
        timeout: profile().timeouts.agentIdle,
      })
      expect(completed, 'the turn never completed').not.toBeNull()

      const { messages } = await api.getTaskMessages(task.taskId)
      const spawned = messages
        .flatMap((message) => message.content)
        .find((content) => content.toolName === 'Agent')?.toolUseId

      // Asking for the tool does not oblige the model to use it. Say so and
      // skip, rather than passing on an assertion that never ran.
      test.skip(!spawned, 'the agent did not use the Agent tool this run')

      // The delegated work is reachable from the tool call that delegated it.
      const transcript = await api.getSubagentTranscript(task.taskId, spawned!)
      expect(
        transcript.messages.length,
        'a subagent produced no transcript under its tool call',
      ).toBeGreaterThan(0)

      // And it is folded out of the main thread: the conversation shows one
      // delegating step, not the subagent's whole conversation inlined.
      const inlined = messages.filter((message) =>
        transcript.messages.some((sub) => sub.uuid === message.uuid),
      )
      expect(inlined, 'subagent messages must not also appear in the main transcript').toHaveLength(
        0,
      )
    } finally {
      sse.stop()
      await api.stopTask(task.taskId).catch(() => {})
      await api.deleteTask(task.taskId).catch(() => {})
    }
  })
})
