/**
 * Standalone: Task Connections (specs/tasks/01-task-scenarios.md)
 *
 * `GET /api/v1/tasks/{id}/connections` is the per-task counterpart to the admin
 * list, and it is scope-checked exactly like reading the task: whoever can see
 * the task can see what is wired to it, and whoever cannot gets a 404.
 *
 * Scenarios:
 *   TASK-13: A task with no external connection reports none
 *   TASK-14: Connections are readable by whoever can read the task
 */

import { scopedName } from '../../shared/run-scope'
import { expect, requiresSeededUsers, test } from '../fixtures/auth-fixtures'

// ─── TASK-13: A task with no external connection reports none ───────────

test('TASK-13: A task with no external connection reports none', async ({ api }) => {
  const repo = await api.ensureTemplateRepository()
  const task = await api.createVibeTask({
    templateId: repo.id,
    name: 'Connections empty-state task',
  })

  try {
    const connections = await api.getTaskConnections(task.taskId)
    expect(connections).toEqual([])
  } finally {
    await api.stopTask(task.taskId).catch(() => {})
    await api.deleteTask(task.taskId).catch(() => {})
  }
})

// ─── TASK-14: Connections are readable by whoever can read the task ─────

test.describe('Standalone: Connections access', () => {
  requiresSeededUsers()

  test('TASK-14: Connections are readable by whoever can read the task', async ({ alice, bob }) => {
    const repo = await alice.createRepository({
      name: scopedName('Connections Scope Repo'),
      kind: 'template',
      scope: 'user',
    })
    const task = await alice.createVibeTask({
      templateId: repo.id,
      name: 'Scoped connections task',
    })

    try {
      // The owner reads it — an empty list, not an admin-only refusal.
      expect(await alice.getTaskConnections(task.taskId)).toEqual([])

      // Someone who cannot see the task cannot see its connections either,
      // and hears the same answer reading the task itself would give.
      const response = await bob.getTaskConnectionsRaw(task.taskId)
      expect(response.status).toBe(404)
    } finally {
      await alice.stopTask(task.taskId).catch(() => {})
      await alice.deleteTask(task.taskId).catch(() => {})
      await alice.deleteRepository(repo.id).catch(() => {})
    }
  })
})
