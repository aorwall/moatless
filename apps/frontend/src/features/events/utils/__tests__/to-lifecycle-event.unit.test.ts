import { describe, expect, it } from 'vitest'
import type { Event } from '@/lib/types/events'
import { toLifecycleEvent } from '../to-lifecycle-event'

/** Build a raw stream event; `any` is allowed in tests, so we cast loosely. */
function makeRaw(overrides: Record<string, unknown>): Event {
  return {
    eventId: 'e1',
    timestamp: '2024-01-01T00:00:00Z',
    isError: false,
    payload: {},
    ...overrides,
  } as unknown as Event
}

describe('toLifecycleEvent', () => {
  it('includes agent, task, taskTurn, sandbox, and server events', () => {
    for (const type of ['agent', 'task', 'taskTurn', 'sandbox', 'server']) {
      const result = toLifecycleEvent(makeRaw({ type, subType: 'x' }))
      expect(result, `${type} should map`).not.toBeNull()
      expect(result?.type).toBe(type)
    }
  })

  it('drops the live-only UI streams (resources/navigation/preview)', () => {
    for (const type of ['resources', 'navigation', 'preview']) {
      expect(toLifecycleEvent(makeRaw({ type })), `${type} should be dropped`).toBeNull()
    }
  })

  it('surfaces the role for agent.message object payloads instead of [object Object]', () => {
    const result = toLifecycleEvent(
      makeRaw({
        type: 'agent',
        subType: 'message',
        agentType: 'claude-code-cli',
        payload: { message: { role: 'assistant', content: [{ type: 'text', text: 'hi' }] } },
      }),
    )
    expect(result?.message).toBe('assistant message')
    expect(result?.agentType).toBe('claude-code-cli')
  })

  it('passes through string messages verbatim', () => {
    const result = toLifecycleEvent(
      makeRaw({ type: 'sandbox', subType: 'ready', payload: { message: 'Ready' } }),
    )
    expect(result?.message).toBe('Ready')
    expect(result?.agentType).toBeUndefined()
  })

  it('preserves sandbox enrichment fields (node, exitCode, pendingSeconds)', () => {
    const result = toLifecycleEvent(
      makeRaw({
        type: 'sandbox',
        subType: 'stopped',
        payload: { nodeName: 'i-abc', exitCode: 137, pendingSeconds: 12, lifecycleEvent: 'fatal' },
      }),
    )
    expect(result?.nodeName).toBe('i-abc')
    expect(result?.exitCode).toBe(137)
    expect(result?.pendingSeconds).toBe(12)
  })
})
