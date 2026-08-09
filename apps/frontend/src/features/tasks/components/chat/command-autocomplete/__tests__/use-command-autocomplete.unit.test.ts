import { act, renderHook } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import type { Suggestion } from '../use-command-autocomplete'
import { useCommandAutocomplete } from '../use-command-autocomplete'

const e2e: Suggestion = {
  kind: 'skill',
  name: 'e2e-testing',
  description: 'Run the E2E suite',
  source: 'workspace',
}
const moat: Suggestion = {
  kind: 'skill',
  name: 'moat-cli',
  description: 'Drive the moat CLI',
  source: 'workspace',
}

function props(overrides: Partial<Parameters<typeof useCommandAutocomplete>[0]> = {}) {
  return {
    value: '',
    onValueChange: vi.fn(),
    onSelectSkill: vi.fn(),
    suggestions: [e2e, moat],
    enabled: true,
    ...overrides,
  }
}

describe('useCommandAutocomplete', () => {
  beforeEach(() => {
    vi.clearAllMocks()
  })

  it('opens on a leading "/" and filters by the typed term', () => {
    const { result } = renderHook(() => useCommandAutocomplete(props()))

    act(() => {
      result.current.handleInputChange('/e2e', 4)
    })

    expect(result.current.isOpen).toBe(true)
    expect(result.current.filteredCommands).toEqual([e2e])
  })

  it('stays closed when "/" is not the first character', () => {
    const { result } = renderHook(() => useCommandAutocomplete(props()))

    act(() => {
      result.current.handleInputChange('fix the /e2e suite', 18)
    })

    expect(result.current.isOpen).toBe(false)
  })

  /// Selecting activates the skill as a field — the "/name " text the old
  /// slash-command mechanism relied on is never written.
  it('activates the skill and strips the trigger instead of writing "/name "', () => {
    const p = props({ value: '/e2e' })
    const { result } = renderHook(() => useCommandAutocomplete(p))

    act(() => {
      result.current.selectCommand(e2e)
    })

    expect(p.onSelectSkill).toHaveBeenCalledWith('e2e-testing')
    expect(p.onValueChange).toHaveBeenCalledWith('')
    expect(p.onValueChange).not.toHaveBeenCalledWith(expect.stringContaining('/e2e-testing'))
    expect(result.current.isOpen).toBe(false)
  })

  it('keeps what the person typed after the trigger', () => {
    const p = props({ value: '/e2e and then check the logs' })
    const { result } = renderHook(() => useCommandAutocomplete(p))

    act(() => {
      result.current.selectCommand(e2e)
    })

    expect(p.onValueChange).toHaveBeenCalledWith('and then check the logs')
  })

  it('does nothing while disabled', () => {
    const { result } = renderHook(() => useCommandAutocomplete(props({ enabled: false })))

    act(() => {
      result.current.handleInputChange('/e2e', 4)
    })

    expect(result.current.isOpen).toBe(false)
  })
})
