/**
 * Standalone: Task Error Cases
 *
 * Independent tests — each creates and tears down its own resources.
 *
 * Scenarios:
 *   TASK-09.2: Create task without agent type (400)
 *   TASK-09.3: Create task with unknown agent type (400)
 *   TASK-10: Create task with invalid repository (404)
 *   TASK-11: Send message to closed task (409)
 */

import { expect, test } from '../fixtures/api-fixtures'
import { createTaskWithMessage } from '../steps'

// ─── TASK-09.2: Create task without agent type (400) ───────────────────

test('TASK-09.2: Create task without agent type (400)', async ({ api }) => {
  const repo = await api.ensureTemplateRepository()
  const response = await api.createVibeTaskRaw({
    repositoryId: repo.id,
    message: 'This should fail',
  })

  expect(response.status).toBe(400)
  expect((response.body as { error?: string }).error).toContain('agentType is required')
  expect((response.body as { error?: string }).error).toContain('claude-code')
  expect((response.body as { error?: string }).error).toContain('codex')
})

// ─── TASK-09.3: Create task with unknown agent type (400) ───────────────

test('TASK-09.3: Create task with unknown agent type (400)', async ({ api }) => {
  const repo = await api.ensureTemplateRepository()
  const response = await api.createVibeTaskRaw({
    repositoryId: repo.id,
    agentType: 'not-an-agent',
    message: 'This should fail',
  })

  expect(response.status).toBe(400)
  expect((response.body as { error?: string }).error).toContain('unknown agentType')
  expect((response.body as { error?: string }).error).toContain('Available options')
})

// ─── TASK-10: Create task with invalid repository (404) ─────────────────

test('TASK-10: Create task with invalid repository (404)', async ({ api }) => {
  const response = await api.createVibeTaskRaw({
    repositoryId: '00000000-0000-0000-0000-000000000000',
    agentType: 'claude-code',
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
