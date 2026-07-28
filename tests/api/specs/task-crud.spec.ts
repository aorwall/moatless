/**
 * Chain: Task CRUD (task-crud.md)
 *
 * Pure API chain — no sandbox or agent execution needed.
 * Tests task creation, retrieval, updates, tagging, visibility, and deletion.
 *
 * Chain:
 *   TASK-01 → TASK-01.1 → TASK-01.2 → TASK-03 → TASK-03.1 → TASK-04 →
 *   TASK-05 → TASK-05.1 → TASK-06 → TASK-07 → TASK-08 →
 *   TASK-08.1 → TASK-09 → TASK-09.1
 */

import { ownedByThisRun, scopedName } from '../../shared/run-scope'
import { ChainContext } from '../context/chain-context'
import { chainEvents } from '../context/chain-events'
import { expect, test } from '../fixtures/api-fixtures'
import { discardTask } from '../helpers/teardown'
import { createTaskWithMessage } from '../steps'

test.describe('Chain: Task CRUD', () => {
  test.describe.configure({ mode: 'serial' })

  const ctx = new ChainContext()
  const events = chainEvents()

  test.beforeAll(async ({ api }) => {
    const repo = await api.ensureTemplateRepository()
    ctx.repositoryId = repo.id
    ctx.repository = repo
  })

  test.afterAll(async ({ api }) => {
    await discardTask(api, ctx.taskId)
  })

  // ─── TASK-01: Create task with message ──────────────────────────────

  test('TASK-01: Create task with message', async ({ api }) => {
    const result = await createTaskWithMessage(api, ctx.repositoryId!, 'E2E CRUD test task')

    expect(result.task.status).toBe('open')
    expect(result.taskId).toBeTruthy()

    ctx.taskId = result.taskId
    ctx.task = result.task
  })

  // ─── TASK-01.1: Task appears in listing ─────────────────────────────

  test('TASK-01.1: Task appears in listing', async ({ api }) => {
    const result = await api.listTasks()

    // The environment's other tasks are not this test's business — narrow to
    // what this run created before asking whether the task is there.
    const ours = ownedByThisRun(result.items, (t) => t.name)
    expect(ours.some((t) => t.taskId === ctx.taskId)).toBe(true)
  })

  // ─── TASK-01.2: SSE task.created event emitted ──────────────────────

  test('TASK-01.2: SSE task.created event emitted', async () => {
    const event = await events().waitForEvent({
      type: 'task',
      taskId: ctx.taskId,
      timeout: 15_000,
    })

    expect(event).not.toBeNull()
    expect(event!.taskId).toBe(ctx.taskId)
  })

  // ─── TASK-03: Get task by ID ────────────────────────────────────────

  test('TASK-03: Get task by ID', async ({ api }) => {
    const task = await api.getTask(ctx.taskId!)

    expect(task.taskId).toBe(ctx.taskId)
    expect(task.status).toBe('open')
    expect(task.repositoryId).toBeTruthy()
    expect(task.createdAt).toBeTruthy()
  })

  // ─── TASK-03.1: Live status is a resource of its own ────────────────

  test('TASK-03.1: Whether the sandbox is running is asked for separately', async ({ api }) => {
    // A task record that carried these would be read as live when it is not.
    const LIVE_FIELDS = [
      'environmentStatus',
      'agentState',
      'isRunning',
      'sandboxStatus',
      'agentStatus',
      'sandboxError',
    ]

    // Read untyped: a typed command would deserialize the extra fields away,
    // and their absence is the whole assertion.
    const list = await api.cli.raw('GET', '/api/v1/tasks')
    const listed = ((list.body as { items?: Record<string, unknown>[] }).items ?? []).find(
      (item) => item.taskId === ctx.taskId,
    )
    expect(listed, 'the created task is missing from the raw listing').toBeDefined()

    const fetched = (await api.cli.raw('GET', `/api/v1/tasks/${ctx.taskId}`)).body as Record<
      string,
      unknown
    >

    for (const field of LIVE_FIELDS) {
      expect(listed, `a listed task must not carry ${field}`).not.toHaveProperty(field)
      expect(fetched, `a fetched task must not carry ${field}`).not.toHaveProperty(field)
    }

    // Asked for directly, live status answers those three and stops there.
    const live = (await api.cli.raw('GET', `/api/sandbox/v1/tasks/${ctx.taskId}/live-status`))
      .body as Record<string, unknown>
    expect(Object.keys(live).sort()).toEqual(['agentStatus', 'sandboxError', 'sandboxStatus'])

    // The fuller report adds the servers, and still drops the retired names.
    const status = (await api.cli.raw('GET', `/api/sandbox/v1/tasks/${ctx.taskId}/status`))
      .body as Record<string, unknown>
    for (const field of ['sandboxStatus', 'agentStatus', 'sandboxError', 'servers']) {
      expect(status, `the status report must carry ${field}`).toHaveProperty(field)
    }
    for (const field of [
      'environmentStatus',
      'agentState',
      'environmentError',
      'sandboxReady',
      'canExecute',
      'sandboxState',
    ]) {
      expect(status, `the status report must not carry ${field}`).not.toHaveProperty(field)
    }
  })

  // ─── TASK-04: Update task metadata ──────────────────────────────────

  test('TASK-04: Update task metadata', async ({ api }) => {
    const updated = await api.updateTaskMetadata(ctx.taskId!, {
      name: scopedName('Updated CRUD Test Task'),
    })

    expect(updated.name).toBe(scopedName('Updated CRUD Test Task'))

    // Confirm via GET
    const fetched = await api.getTask(ctx.taskId!)
    expect(fetched.name).toBe(scopedName('Updated CRUD Test Task'))
  })

  // ─── TASK-05: Set task tags ─────────────────────────────────────────

  test('TASK-05: Set task tags', async ({ api }) => {
    await api.setTaskTags(ctx.taskId!, ['e2e', 'crud-test'])

    const task = await api.getTask(ctx.taskId!)
    expect(task.tags).toBeDefined()
    expect(task.tags!.length).toBeGreaterThanOrEqual(2)

    const tagNames = task.tags!.map((t) => t.name)
    expect(tagNames).toContain('e2e')
    expect(tagNames).toContain('crud-test')
  })

  // ─── TASK-05.1: Clear tags ──────────────────────────────────────────

  test('TASK-05.1: Clear tags', async ({ api }) => {
    await api.setTaskTags(ctx.taskId!, [])

    const task = await api.getTask(ctx.taskId!)
    expect(task.tags ?? []).toHaveLength(0)
  })

  // ─── TASK-06: Update task visibility ────────────────────────────────

  test('TASK-06: Update task visibility', async ({ api }) => {
    const updated = await api.updateVisibility(ctx.taskId!, 'public')

    expect(updated.visibility).toBe('public')

    // Confirm via GET
    const fetched = await api.getTask(ctx.taskId!)
    expect(fetched.visibility).toBe('public')
  })

  // ─── TASK-07: List tasks with filters ───────────────────────────────

  test('TASK-07: List tasks with filters', async ({ api }) => {
    const result = await api.listTasks({ status: 'open' })

    const ours = ownedByThisRun(result.items, (t) => t.name)
    expect(ours.some((t) => t.taskId === ctx.taskId)).toBe(true)

    // The filter is the thing under test, so every row it returned has to
    // honour it — including rows this run did not create.
    for (const task of result.items) {
      expect(task.status).toBe('open')
    }
  })

  // ─── TASK-08: Close task ────────────────────────────────────────────

  test('TASK-08: Close task', async ({ api }) => {
    const closed = await api.closeTask(ctx.taskId!)

    expect(closed.status).toBe('closed')

    // Confirm via GET
    const fetched = await api.getTask(ctx.taskId!)
    expect(fetched.status).toBe('closed')
  })

  // ─── TASK-08.1: SSE task.closed event ───────────────────────────────

  test('TASK-08.1: SSE task.closed event', async () => {
    const event = await events().waitForEvent({
      type: 'task',
      subType: 'closed',
      taskId: ctx.taskId,
      timeout: 5000,
    })

    const taskEvents = events().getEventsForTask(ctx.taskId!)
    const hasClosedEvent = taskEvents.some(
      (e) => e.type === 'task' && (e.subType === 'closed' || e.subType === 'updated'),
    )
    expect(event !== null || hasClosedEvent).toBe(true)
  })

  // ─── TASK-09: Delete task ───────────────────────────────────────────

  test('TASK-09: Delete task', async ({ api }) => {
    const response = await api.deleteTaskRaw(ctx.taskId!)

    expect(response.status).toBeLessThan(300)
  })

  // ─── TASK-09.1: Task removed from listing ───────────────────────────

  test('TASK-09.1: Task removed from listing', async ({ api }) => {
    const result = await api.listTasks()

    const ours = ownedByThisRun(result.items, (t) => t.name)
    expect(ours.some((t) => t.taskId === ctx.taskId)).toBe(false)

    // Prevent afterAll from trying to delete again
    ctx.taskId = undefined
  })
})
