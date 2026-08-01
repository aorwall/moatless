/**
 * Chain 1: Subscription Loop CRUD
 * Chain 2: Schedule Loop CRUD
 *
 * Pure API chains — subscription and schedule loop creation, retrieval, updates, deletion.
 *
 * A subscription loop fires on an inbound adapter event; a schedule loop fires
 * on a cron expression it carries. Both are the same resource on the same
 * endpoint, which is why they share a file.
 *
 * Chain 1:
 *   INT-01 → INT-01.1 → INT-02 → INT-03 → INT-04 → INT-05 → INT-05.1
 * Chain 2:
 *   INT-07 → INT-07.1 → INT-07.2 → INT-07.3
 */

import type { Schedule } from '@moatless/api-client/sandbox/model'
import { scopedName } from '../../shared/run-scope'
import { ChainContext } from '../context/chain-context'
import { expect, test } from '../fixtures/api-fixtures'
import { createWebhookTriggerFixture } from '../steps'

test.describe('Chain 1: Task Route and Trigger CRUD', () => {
  test.describe.configure({ mode: 'serial' })

  const ctx = new ChainContext()

  test.beforeAll(async ({ api }) => {
    const repo = await api.ensureTemplateRepository()
    ctx.repositoryId = repo.id
    ctx.repository = repo
  })

  test.afterAll(async ({ api }) => {
    const triggerId = ctx.get<string>('triggerId')
    if (triggerId) {
      await api.deleteTaskTriggerFixture(triggerId).catch(() => {})
    }
  })

  // ─── INT-01: Create webhook subscription loop fixture ───────────────────

  test('INT-01: Create webhook task trigger fixture', async ({ api }) => {
    const sourceId = `crud-test-${Date.now()}`
    const result = await createWebhookTriggerFixture(api, ctx.repositoryId!, {
      sourceId,
      sourceName: 'CRUD Test Webhook',
    })

    expect(result.config.active).toBe(true)
    expect(result.triggerId).toBeTruthy()
    expect(result.config.adapterKind).toBe('webhook')
    expect(result.config.repositoryId).toBe(ctx.repositoryId)
    // The matcher is what the inbound event is routed by, so it has to be
    // the one asked for — the connection is created from the same string.
    expect(result.config.connectionId).toBeTruthy()
    expect(result.config.sourceId).toContain(sourceId)

    ctx.set('sourceId', result.config.sourceId)

    ctx.set('triggerId', result.triggerId)
    ctx.set('config', result.config)
  })

  // ─── INT-01.1: Config appears in listing ──────────────────────────

  test('INT-01.1: Config appears in listing', async ({ api }) => {
    const configs = await api.listTaskTriggerFixtures()

    const found = configs.some((c) => c.id === ctx.get<string>('triggerId'))
    expect(found).toBe(true)
  })

  // ─── INT-02: Get subscription loop fixture by ID ────────────────────────

  test('INT-02: Get task trigger fixture by ID', async ({ api }) => {
    const config = await api.getTaskTriggerFixture(ctx.get<string>('triggerId')!)

    expect(config.id).toBe(ctx.get<string>('triggerId'))
    expect(config.adapterKind).toBe('webhook')
    expect(config.routingMode).toBe('by_subject')
    expect(config.active).toBe(true)
  })

  // ─── INT-03: Update subscription loop fixture ───────────────────────────

  test('INT-03: Update task trigger fixture', async ({ api }) => {
    const updated = await api.updateTaskTriggerFixture(ctx.get<string>('triggerId')!, {
      sourceName: 'Updated CRUD Webhook',
      promptTemplate: 'Please help: {message}',
      taskName: 'Updated webhook task',
    })

    expect(updated.sourceName).toBe(scopedName('Updated CRUD Webhook'))
    expect(updated.promptTemplate).toBe('Please help: {message}')
    // The name the routed task will be given, which is a separate field from
    // the prompt it will be started with.
    expect(updated.taskName).toBe('Updated webhook task')
    expect(updated.routingMode).toBe('by_subject')
  })

  // ─── INT-04: Deactivate subscription loop fixture ───────────────────────

  test('INT-04: Deactivate task trigger fixture', async ({ api }) => {
    const updated = await api.updateTaskTriggerFixture(ctx.get<string>('triggerId')!, {
      active: false,
    })

    expect(updated.active).toBe(false)

    // Still appears in listing
    const configs = await api.listTaskTriggerFixtures()
    const found = configs.some((c) => c.id === ctx.get<string>('triggerId'))
    expect(found).toBe(true)
  })

  // ─── INT-05: Delete subscription loop fixture ───────────────────────────

  test('INT-05: Delete task trigger fixture', async ({ api }) => {
    const response = await api.deleteTaskTriggerFixtureRaw(ctx.get<string>('triggerId')!)

    expect(response.status).toBe(204)
  })

  // ─── INT-05.1: Config removed from listing ───────────────────────

  test('INT-05.1: Config removed from listing', async ({ api }) => {
    const configs = await api.listTaskTriggerFixtures()

    const found = configs.some((c) => c.id === ctx.get<string>('triggerId'))
    expect(found).toBe(false)

    ctx.set('triggerId', undefined)
  })
})

// =============================================================================
// Chain 2: Schedule Loop CRUD
// =============================================================================

test.describe('Chain 2: Schedule Loop CRUD', () => {
  test.describe.configure({ mode: 'serial' })

  const ctx = new ChainContext()
  // Far enough out that the schedule cannot fire during the run. The loop is
  // also created paused; this test is about the record, and a schedule that
  // can fire on a shared cluster keeps spawning tasks if the run is
  // interrupted before the delete.
  const cron = '0 0 0 1 1 * 2099'
  const updatedCron = '0 0 0 1 6 * 2098'

  test.beforeAll(async ({ api }) => {
    const repo = await api.ensureTemplateRepository()
    ctx.repositoryId = repo.id
  })

  test.afterAll(async ({ api }) => {
    const loopId = ctx.get<string>('loopId')
    if (loopId) {
      await api.deleteLoop(loopId).catch(() => {})
    }
  })

  // ─── INT-07: Create a schedule loop ─────────────────────────────────────

  test('INT-07: Create a schedule loop', async ({ api }) => {
    const loop = await api.createScheduleLoop({
      name: 'scheduled-crud',
      repositoryId: ctx.repositoryId!,
      cronExpression: cron,
      messageTemplate: 'Scheduled smoke message',
      timezone: 'UTC',
      taskName: 'Scheduled smoke task',
      prompt: 'Scheduled prompt: {message}',
      agentType: 'codex',
    })

    expect(loop.kind).toBe('schedule')
    expect(loop.active).toBe(false)
    expect(loop.config.repositoryId).toBe(ctx.repositoryId)
    expect(loop.config.routingMode).toBe('by_subject')
    expect(loop.config.taskName).toBe(scopedName('Scheduled smoke task'))
    expect(loop.config.prompt).toBe('Scheduled prompt: {message}')

    const source = loop.source as Schedule
    expect(source.cronExpression).toBe(cron)
    expect(source.timezone).toBe('UTC')
    expect(source.messageTemplate).toBe('Scheduled smoke message')
    // Nothing has fired yet, and the next firing is computed on creation.
    expect(source.lastFiredAt ?? null).toBeNull()
    expect(Date.parse(source.nextFireAt)).toBeGreaterThan(Date.now())

    ctx.set('loopId', loop.id)
    ctx.set('nextFireAt', source.nextFireAt)
  })

  // ─── INT-07.1: Schedule loop appears in the schedule listing ────────────

  test('INT-07.1: Schedule loop appears in the schedule listing', async ({ api }) => {
    const loops = await api.listLoops({ kind: 'schedule' })

    expect(loops.some((loop) => loop.id === ctx.get<string>('loopId'))).toBe(true)
  })

  // ─── INT-07.2: Updating the cron expression moves the next firing ───────

  test('INT-07.2: Updating the cron expression moves the next firing', async ({ api }) => {
    const updated = await api.updateLoop(ctx.get<string>('loopId')!, {
      cronExpression: updatedCron,
      timezone: 'Europe/Stockholm',
      messageTemplate: 'Updated scheduled smoke message',
    })

    const source = updated.source as Schedule
    expect(source.cronExpression).toBe(updatedCron)
    expect(source.timezone).toBe('Europe/Stockholm')
    expect(source.messageTemplate).toBe('Updated scheduled smoke message')
    // The point of the update: the schedule is recomputed, not left on the
    // firing time the old expression produced.
    expect(source.nextFireAt).not.toBe(ctx.get<string>('nextFireAt'))
    expect(Date.parse(source.nextFireAt)).toBeGreaterThan(Date.now())
  })

  // ─── INT-07.3: Delete the schedule loop ─────────────────────────────────

  test('INT-07.3: Delete the schedule loop', async ({ api }) => {
    await api.deleteLoop(ctx.get<string>('loopId')!)

    const loops = await api.listLoops({ kind: 'schedule' })
    expect(loops.some((loop) => loop.id === ctx.get<string>('loopId'))).toBe(false)

    ctx.set('loopId', undefined)
  })
})
