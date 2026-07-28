/**
 * Standalone: Feedback (specs/tasks/01-task-scenarios.md)
 *
 * Feedback is filed against a task and optionally narrowed to one message, or to
 * one tool call inside that message. Only the user-filed path is covered here —
 * agent-filed feedback needs a sandbox-issued token, which is the agent chain's
 * business.
 *
 * Scenarios:
 *   TASK-20: Feedback at task, message and tool-call level
 *   TASK-21: A tool call named without its message is rejected
 *   TASK-22: Preview-failure feedback carries its metadata
 */

import type { TaskResponse } from '@moatless/api-client/sandbox/model'
import type { MoatApi } from '../../shared/moat-api'
import { expect, test } from '../fixtures/api-fixtures'

/** A task with no message, so nothing runs while the feedback is filed. */
async function taskForFeedback(api: MoatApi, name: string): Promise<TaskResponse> {
  const repo = await api.ensureTemplateRepository()
  return api.createVibeTask({ templateId: repo.id, name })
}

async function discard(api: MoatApi, task: TaskResponse): Promise<void> {
  await api.stopTask(task.taskId).catch(() => {})
  await api.deleteTask(task.taskId).catch(() => {})
}

// ─── TASK-20: Feedback at task, message and tool-call level ─────────────

test('TASK-20: Feedback at task, message and tool-call level', async ({ api }) => {
  const task = await taskForFeedback(api, 'Feedback surfaces task')

  try {
    const taskFeedback = await api.submitFeedback({
      taskId: task.taskId,
      surface: 'task',
      comment: 'task is great',
    })
    expect(taskFeedback.surface).toBe('task')
    // Filed through a user credential, so it is the user speaking, not the agent.
    expect(taskFeedback.source).toBe('user')
    expect(taskFeedback.taskId).toBe(task.taskId)
    expect(taskFeedback.messageUuid ?? null).toBeNull()
    expect(taskFeedback.toolUseId ?? null).toBeNull()

    const messageFeedback = await api.submitFeedback({
      taskId: task.taskId,
      surface: 'message',
      messageUuid: 'msg-uuid-1',
      comment: 'this answer was unhelpful',
    })
    expect(messageFeedback.surface).toBe('message')
    expect(messageFeedback.messageUuid).toBe('msg-uuid-1')

    const toolFeedback = await api.submitFeedback({
      taskId: task.taskId,
      surface: 'tool_call',
      messageUuid: 'msg-uuid-1',
      toolUseId: 'tu-1',
      comment: 'this Bash invocation was unnecessary',
    })
    expect(toolFeedback.surface).toBe('tool_call')
    expect(toolFeedback.toolUseId).toBe('tu-1')

    const listed = await api.listTaskFeedback(task.taskId)
    const ids = listed.items.map((item) => item.feedbackId)
    expect(ids).toContain(taskFeedback.feedbackId)
    expect(ids).toContain(messageFeedback.feedbackId)
    expect(ids).toContain(toolFeedback.feedbackId)
  } finally {
    await discard(api, task)
  }
})

// ─── TASK-21: A tool call named without its message is rejected ─────────

test('TASK-21: A tool call named without its message is rejected', async ({ api }) => {
  const task = await taskForFeedback(api, 'Feedback rejection task')

  try {
    const response = await api.submitFeedbackRaw({
      taskId: task.taskId,
      surface: 'tool_call',
      toolUseId: 'tu-orphan',
      comment: 'broken scope',
    })

    expect(response.status).toBeGreaterThanOrEqual(400)
    expect(response.status).toBeLessThan(500)
  } finally {
    await discard(api, task)
  }
})

// ─── TASK-22: Preview-failure feedback carries its metadata ─────────────

test('TASK-22: Preview-failure feedback carries its metadata', async ({ api }) => {
  const task = await taskForFeedback(api, 'Preview failure feedback task')

  try {
    const feedback = await api.submitFeedback({
      taskId: task.taskId,
      surface: 'preview_failure',
      comment: 'preview crashed',
      metadata: {
        preview_error: 'ECONNREFUSED',
        server_name: 'preview',
      },
    })

    expect(feedback.surface).toBe('preview_failure')
    const metadata = feedback.metadata as Record<string, unknown>
    expect(metadata.preview_error).toBe('ECONNREFUSED')
  } finally {
    await discard(api, task)
  }
})
