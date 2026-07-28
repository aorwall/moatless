/**
 * Standalone: Messages (specs/tasks/01-task-scenarios.md)
 *
 * The conversation accumulates. Creating a task with a message and then sending
 * another leaves both readable — no agent has to run for that to hold.
 *
 * Scenarios:
 *   TASK-12: Send a follow-up message and read the conversation back
 */

import { expect, test } from '../fixtures/api-fixtures'

test('TASK-12: Send a follow-up message and read the conversation back', async ({ api }) => {
  const repo = await api.ensureTemplateRepository()
  const task = await api.createVibeTask({
    templateId: repo.id,
    name: 'Message listing task',
    message: 'Initial message',
  })

  try {
    await api.sendMessage(task.taskId, 'Follow-up message')

    const messages = await api.getTaskMessages(task.taskId)
    expect(messages.messages.length).toBeGreaterThan(0)
  } finally {
    await api.stopTask(task.taskId).catch(() => {})
    await api.deleteTask(task.taskId).catch(() => {})
  }
})
