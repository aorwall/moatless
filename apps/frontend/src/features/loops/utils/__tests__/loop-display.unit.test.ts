import type { Loop } from '@moatless/api-client'
import { describe, expect, it } from 'vitest'
import { getSourceLabel } from '@/features/loops/utils/loop-display'

function makeLoop(source: Loop['source']): Loop {
  return { name: 'Alerts', source } as Loop
}

describe('getSourceLabel', () => {
  it('names the provider a subscription listens on, and the subject on it', () => {
    const label = getSourceLabel(
      makeLoop({ adapterKind: 'slack', sourceMatcher: 'C0000000002' } as Loop['source']),
    )

    // The provider belongs in the list, not folded into the Loop's name.
    expect(label).toBe('Slack · C0000000002')
  })

  it('prefers a source name over the raw matcher', () => {
    const label = getSourceLabel(
      makeLoop({
        adapterKind: 'github',
        sourceMatcher: 'soaplabs/moatless',
        sourceName: '#alerts-critical',
      } as Loop['source']),
    )

    expect(label).toBe('GitHub · #alerts-critical')
  })

  it('labels a schedule by the cron it fires on', () => {
    const label = getSourceLabel(makeLoop({ cronExpression: '0 0 3 * * * *' } as Loop['source']))

    expect(label).toBe('Schedule · 0 0 3 * * * *')
  })

  it('falls back to Manual for a loop with neither source', () => {
    expect(getSourceLabel(makeLoop({} as Loop['source']))).toBe('Manual')
  })
})
