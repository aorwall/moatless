import { describe, expect, it } from 'vitest'
import type { SandboxEvent } from '../../types'
import { describeEvent, formatDurationSeconds, getSourceLabel } from '../event-messages'

function makeEvent(overrides: Partial<SandboxEvent> = {}): SandboxEvent {
  return {
    eventId: 'e1',
    type: 'sandbox',
    subType: 'stopped',
    timestamp: '2024-01-01T00:00:00Z',
    isError: false,
    payload: {},
    ...overrides,
  }
}

describe('describeEvent', () => {
  it('keys off payload.lifecycleEvent for sandbox events', () => {
    const evt = makeEvent({
      subType: 'error',
      isError: true,
      payload: { lifecycleEvent: 'pod_evicted' },
    })
    const { label, tone } = describeEvent(evt)
    expect(label).toBe('Evicted')
    // Eviction is recoverable — amber, NOT red error.
    expect(tone).toBe('recovering')
  })

  it('renders idle_reaped as a neutral stop, not an error', () => {
    const { label, tone } = describeEvent(
      makeEvent({ subType: 'stopped', payload: { lifecycleEvent: 'idle_reaped' } }),
    )
    expect(label).toBe('Stopped (idle)')
    expect(tone).toBe('neutral')
  })

  it('renders crashes and OOM as red errors', () => {
    expect(
      describeEvent(makeEvent({ payload: { lifecycleEvent: 'container_oom_killed' } })).tone,
    ).toBe('error')
    expect(describeEvent(makeEvent({ payload: { lifecycleEvent: 'fatal' } })).tone).toBe('error')
  })

  it('renders pod_ready as ok', () => {
    expect(describeEvent(makeEvent({ payload: { lifecycleEvent: 'pod_ready' } })).tone).toBe('ok')
  })

  it('covers the new Phase 2 subtypes', () => {
    expect(
      describeEvent(makeEvent({ payload: { lifecycleEvent: 'scheduling_blocked' } })).tone,
    ).toBe('recovering')
    expect(describeEvent(makeEvent({ payload: { lifecycleEvent: 'startup_slow' } })).tone).toBe(
      'recovering',
    )
    const turn = describeEvent(makeEvent({ type: 'taskTurn', subType: 'turn_interrupted' }))
    expect(turn.label).toBe('Turn interrupted')
    expect(turn.tone).toBe('recovering')
  })

  it('builds metadata chips from enrichment fields', () => {
    const { chips } = describeEvent(
      makeEvent({
        payload: { lifecycleEvent: 'pod_evicted' },
        nodeName: 'ip-10-0-1-42',
        disruptionReason: 'PreemptionByScheduler',
        initiator: 'eviction',
        exitCode: 137,
        terminatedReason: 'Evicted',
      }),
    )
    expect(chips).toEqual([
      { label: 'node', value: 'ip-10-0-1-42' },
      { label: 'cause', value: 'PreemptionByScheduler' },
      { label: 'by', value: 'eviction' },
      { label: 'exit', value: '137' },
      { label: 'reason', value: 'Evicted' },
    ])
  })

  it('renders a pending-duration chip on pod_scheduled', () => {
    const { chips } = describeEvent(
      makeEvent({ payload: { lifecycleEvent: 'pod_scheduled' }, pendingSeconds: 364 }),
    )
    expect(chips).toContainEqual({ label: 'pending', value: '6m 4s' })
  })

  it('renders old events without enrichment fields unchanged (no chips)', () => {
    const { label, tone, chips } = describeEvent(makeEvent({ subType: 'ready', payload: {} }))
    expect(label).toBe('Ready')
    expect(tone).toBe('ok')
    expect(chips).toEqual([])
  })

  it('falls back to a humanized label and error tone for unknown flagged events', () => {
    const { label, tone } = describeEvent(
      makeEvent({ subType: 'weird_new_thing', isError: true, payload: {} }),
    )
    expect(label).toBe('weird new thing')
    expect(tone).toBe('error')
  })
})

describe('getSourceLabel', () => {
  it('labels taskTurn events as "turn"', () => {
    expect(getSourceLabel(makeEvent({ type: 'taskTurn' }))).toBe('turn')
  })

  it('labels server events with the server name', () => {
    expect(getSourceLabel(makeEvent({ type: 'server', serverName: 'preview' }))).toBe('preview')
  })
})

describe('formatDurationSeconds', () => {
  it('formats sub-minute, minute, and hour ranges', () => {
    expect(formatDurationSeconds(42)).toBe('42s')
    expect(formatDurationSeconds(364)).toBe('6m 4s')
    expect(formatDurationSeconds(120)).toBe('2m')
    expect(formatDurationSeconds(3661)).toBe('1h 1m')
  })

  it('clamps negatives to zero', () => {
    expect(formatDurationSeconds(-5)).toBe('0s')
  })
})
