/**
 * Standalone: Task Error Cases
 *
 * Independent tests — each creates and tears down its own resources.
 *
 * Scenarios:
 *   TASK-09.2: Create task without agent profile (400)
 *   TASK-09.3: Create task with unknown agent profile (400)
 *   TASK-10: Create task in a missing workspace (404)
 *   TASK-11: Send message to closed task (409)
 *   TASK-12.1: Switch to the current agent profile (200, unchanged)
 *   TASK-12.2: Switch to an unknown agent profile (400)
 *   TASK-12.3: Switch to a tmux profile in a non-tmux Sandbox (409)
 */

import { missingCapability } from '../../shared/profile'
import { expect, test } from '../fixtures/api-fixtures'
import { createTaskWithMessage, createTaskWithoutMessage } from '../steps'

// ─── TASK-09.2: Create task without agent profile (400) ───────────────────

test('TASK-09.2: Create task without agent profile (400)', async ({ api }) => {
  const workspace = await api.ensureTemplateWorkspace()
  const response = await api.createVibeTaskRaw({
    workspaceId: workspace.id,
    message: 'This should fail',
  })

  expect(response.status).toBe(400)
  expect((response.body as { error?: string }).error).toContain('agentProfile is required')
  expect((response.body as { error?: string }).error).toContain('claude-code')
  expect((response.body as { error?: string }).error).toContain('codex')
})

// ─── TASK-09.3: Create task with unknown agent profile (400) ───────────────

test('TASK-09.3: Create task with unknown agent profile (400)', async ({ api }) => {
  const workspace = await api.ensureTemplateWorkspace()
  const response = await api.createVibeTaskRaw({
    workspaceId: workspace.id,
    agentProfile: 'not-an-agent',
    message: 'This should fail',
  })

  expect(response.status).toBe(400)
  expect((response.body as { error?: string }).error).toContain('unknown agentProfile')
  expect((response.body as { error?: string }).error).toContain('Available options')
})

// ─── TASK-10: Create task in a missing workspace (404) ─────────────────

test('TASK-10: Create task in a missing workspace (404)', async ({ api }) => {
  const response = await api.createVibeTaskRaw({
    workspaceId: '00000000-0000-0000-0000-000000000000',
    agentProfile: 'claude-code',
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

// ─── TASK-12: Agent profile switch refusals (no agent run) ─────────────

test('TASK-12.1: Switching to the current profile changes nothing (200)', async ({ api }) => {
  const repo = await api.ensureTemplateRepository()
  const { taskId } = await createTaskWithoutMessage(api, repo.id)
  try {
    const response = await api.switchProfileRaw(taskId, 'claude-code')
    expect(response.status).toBe(200)
    expect((response.body as { agentProfile?: string }).agentProfile).toBe('claude-code')
  } finally {
    await api.deleteTask(taskId).catch(() => {})
  }
})

test('TASK-12.2: Switching to an unknown profile (400)', async ({ api }) => {
  const repo = await api.ensureTemplateRepository()
  const { taskId } = await createTaskWithoutMessage(api, repo.id)
  try {
    const response = await api.switchProfileRaw(taskId, 'not-an-agent')
    expect(response.status).toBe(400)
    expect((response.body as { error?: string }).error).toContain('unknown agentProfile')
  } finally {
    await api.deleteTask(taskId).catch(() => {})
  }
})

test('TASK-12.3: Switching to a tmux profile in a non-tmux Sandbox (409)', async ({ api }) => {
  const reason = missingCapability('claudeCredentials')
  test.skip(reason !== null, reason ?? '')
  await api.storeHarnessCredentials()
  const repo = await api.ensureTemplateRepository()
  const { taskId } = await createTaskWithoutMessage(api, repo.id)
  try {
    const response = await api.switchProfileRaw(taskId, 'claude-code-tui')
    expect(response.status).toBe(409)
    expect((response.body as { error?: string }).error).toContain('This Sandbox cannot run')
  } finally {
    await api.deleteTask(taskId).catch(() => {})
  }
})
