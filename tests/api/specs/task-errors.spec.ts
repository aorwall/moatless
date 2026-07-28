/**
 * Standalone: Task Error Cases (task-crud.md)
 *
 * Independent tests — each creates and tears down its own resources.
 *
 * Scenarios:
 *   TASK-10: Create task with invalid repository (404)
 *   TASK-11: Send message to closed task (409)
 */

import { expect, test } from '../fixtures/api-fixtures'
import { createTaskWithMessage } from '../steps'

// ─── TASK-10: Create task with invalid repository (404) ─────────────────

test('TASK-10: Create task with invalid repository (404)', async ({ api }) => {
  const response = await api.createVibeTaskRaw({
    repositoryId: '00000000-0000-0000-0000-000000000000',
    message: 'This should fail',
  })

  expect(response.status).toBe(404)
  expect(response.body).toBeDefined()
})

// ─── TASK-11: Send message to closed task (rejected — 409) ──────────────

test('TASK-11: Send message to closed task (409)', async ({ api }) => {
  // Setup: create a task, then close it
  const repo = await api.ensureTemplateRepository()
  const { taskId } = await createTaskWithMessage(api, repo.id, 'Temp task for close test')

  await api.closeTask(taskId)

  // Act: send message to the closed task — a closed task is terminal
  const response = await api.sendMessageRaw(taskId, 'This should fail because the task is closed')

  expect(response.status).toBe(409)

  // Cleanup
  await api.deleteTask(taskId).catch(() => {})
})
