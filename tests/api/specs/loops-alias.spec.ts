/**
 * Standalone: Loop path alias (specs/channels/01-channel-scenarios.md)
 *
 * `/api/v1/loops` is the documented path. `/api/v1/triggers` is the older name,
 * kept working undocumented for clients that still call it. They are the same
 * handlers, so the same operation has to answer identically on either — which is
 * a claim only a byte-for-byte comparison can make.
 *
 * Both paths are reached through the raw call: the CLI speaks the documented one
 * only, and the point here is the path, not the client.
 *
 * Scenarios:
 *   INT-06: The loops path and its triggers alias answer identically
 */

import { expect, test } from '../fixtures/api-fixtures'
import { createWebhookTriggerFixture } from '../steps'

test('INT-06: The loops path and its triggers alias answer identically', async ({ api }) => {
  const repo = await api.ensureTemplateRepository()
  const { triggerId } = await createWebhookTriggerFixture(api, repo.id, {
    sourceId: `loop-alias-${Date.now()}`,
  })

  try {
    const viaLoops = await api.getLoopVia('loops', triggerId)
    const viaTriggers = await api.getLoopVia('triggers', triggerId)
    expect(viaTriggers).toEqual(viaLoops)

    const loopsList = await api.listLoopsVia('loops', {
      kind: 'adapter_event',
      adapterKind: 'webhook',
    })
    const triggersList = await api.listLoopsVia('triggers', {
      kind: 'adapter_event',
      adapterKind: 'webhook',
    })

    const has = (list: unknown[]): boolean =>
      list.some((entry) => (entry as { id?: string }).id === triggerId)
    expect(has(loopsList)).toBe(true)
    expect(has(triggersList)).toBe(true)
    expect(triggersList).toEqual(loopsList)
  } finally {
    await api.deleteLoop(triggerId).catch(() => {})
  }
})
