/**
 * Standalone: Agent tool interaction
 *
 * What the agent does with tools that only shows up end to end: pausing a turn
 * to ask the user a question. It drives a real agent, so it needs agent
 * credentials.
 *
 * Scenarios:
 *   AGENT-07: Answering the agent's question resumes the same turn
 */

import { EventStream } from '../../shared/event-stream'
import { missingCapability, profile } from '../../shared/profile'
import { expect, test } from '../fixtures/api-fixtures'
import { progress } from '../helpers/progress'
import { waitForPendingToolCall, waitForSandboxReady } from '../steps'

const reason = missingCapability('claudeCredentials')

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
      // The agent only has AskUserQuestion where someone is watching for it.
      headless: false,
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
      progress(
        outcome.toolUseId
          ? 'AGENT-07: the question was parked and shown'
          : 'AGENT-07: the agent finished without asking',
      )

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
