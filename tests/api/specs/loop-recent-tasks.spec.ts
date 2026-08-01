/**
 * Standalone: Loop recent tasks
 *
 * `GET /api/v1/loops/{id}/tasks` answers from the task's own loop id, not
 * from a binding. That distinction is the whole point: a schedule loop never
 * creates a binding, so a listing driven by bindings would report that a loop
 * firing every night has produced nothing.
 *
 * Scenarios:
 *   INT-45: A schedule loop's tasks are listed, with no binding
 *   INT-46: A subscription loop's tasks are listed with their binding
 */

import { randomUUID } from 'node:crypto'
import { missingCapability } from '../../shared/profile'
import { expect, test } from '../fixtures/api-fixtures'
import { psql } from '../helpers/psql'
import { discardTask } from '../helpers/teardown'
import { createWebhookTriggerFixture } from '../steps'

// ─── INT-45: A schedule loop's tasks are listed, with no binding ────────

test.describe('Standalone: Schedule loop recent tasks', () => {
  // The schedule runner ticks on its own clock, far slower than a test can
  // wait for, so the row it would write is written directly instead.
  const reason = missingCapability('directDatabase')
  test.skip(reason !== null, reason ?? '')

  test("INT-45: A schedule loop's tasks are listed, with no binding", async ({ api }) => {
    const repo = await api.ensureTemplateRepository()
    const loop = await api.createScheduleLoop({
      name: 'recent-tasks-schedule',
      repositoryId: repo.id,
      // Created paused, and dated far enough out that it could not fire
      // even if it were active — the task row is stamped below instead.
      cronExpression: '0 0 0 1 1 * 2099',
      messageTemplate: `scheduled-tick-${Date.now()}`,
      timezone: 'UTC',
      taskName: 'Scheduled recent-tasks check',
    })
    const taskId = randomUUID()

    try {
      psql(
        'INSERT INTO tasks (task_id, short_id, created_at, last_message_at, agent_type, ' +
          'repository_id, status, total_cost_usd, visibility, scope, team_id, headless, ' +
          `ttl_minutes, inactive_ttl_days, loop_id) VALUES ('${taskId}', ` +
          `'${taskId.slice(0, 8)}', (now() AT TIME ZONE 'UTC')::text, ` +
          `(now() AT TIME ZONE 'UTC')::text, 'claude-code', '${repo.id}', 'open', 0, ` +
          `'private', 'global', 'system', true, 15, 7, '${loop.id}')`,
      )

      const items = await api.listLoopRecentTasks(loop.id)

      expect(items).toHaveLength(1)
      expect(items[0]?.task.taskId).toBe(taskId)
      expect(items[0]?.binding ?? null).toBeNull()
    } finally {
      psql(`DELETE FROM tasks WHERE task_id = '${taskId}'`)
      await api.deleteLoop(loop.id).catch(() => {})
    }
  })
})

// ─── INT-46: A subscription loop's tasks are listed with their binding ──

test("INT-46: A subscription loop's tasks are listed with their binding", async ({ api }) => {
  const repo = await api.ensureTemplateRepository()
  const { loopId } = await createWebhookTriggerFixture(api, repo.id, {
    sourceId: `recent-tasks-adapter-${Date.now()}`,
  })

  const inbound = await api.sendWebhookMessage(loopId, {
    text: 'trigger-recent-binding-test',
  })
  expect(inbound.status).toBe(200)
  const taskId = (inbound.body as { taskId: string }).taskId

  try {
    const items = await api.listLoopRecentTasks(loopId)

    const item = items.find((entry) => entry.task.taskId === taskId)
    expect(item).toBeDefined()
    // An inbound event does create a binding, and it is reported.
    expect(item?.binding?.id).toBeTruthy()
    expect(item?.binding?.subjectKey).toBeTruthy()
  } finally {
    await discardTask(api, taskId)
    await api.deleteLoop(loopId).catch(() => {})
  }
})
