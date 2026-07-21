import {
  type SessionResponse,
  SetupActionKind,
  SetupRequirementId,
  SetupRequirementStatus,
  SetupSeverity,
} from '@moatless/api-client'
import { renderHook } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'

import { useAuth } from '@/features/auth'
import { useSetupStatus } from '../use-setup-status'

const { mockUseQuery } = vi.hoisted(() => ({ mockUseQuery: vi.fn() }))

vi.mock('@tanstack/react-query', () => ({ useQuery: mockUseQuery }))
vi.mock('@/features/auth', () => ({
  authSessionQueryOptions: vi.fn(() => ({})),
  useAuth: vi.fn(),
}))

function mockSession(session: SessionResponse | undefined, isLoading = false) {
  mockUseQuery.mockReturnValue({ data: session, isLoading })
}

function mockIsAdmin(isAdmin: boolean) {
  vi.mocked(useAuth).mockReturnValue({ isAdmin } as unknown as ReturnType<typeof useAuth>)
}

describe('useSetupStatus', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    mockIsAdmin(false)
  })

  describe('when the backend reports no setup block', () => {
    it('treats setup as complete, so no banner flashes on load', () => {
      mockSession({ user: null, expiresAt: null, providers: [] })
      const { result } = renderHook(() => useSetupStatus())
      expect(result.current.isComplete).toBe(true)
      expect(result.current.unmet).toEqual([])
    })

    it('treats a still-loading session as complete', () => {
      mockSession(undefined, true)
      const { result } = renderHook(() => useSetupStatus())
      expect(result.current.isComplete).toBe(true)
    })
  })

  it('surfaces an unmet requirement with copy and a CTA', () => {
    mockSession({
      user: null,
      expiresAt: null,
      providers: [],
      setup: {
        complete: false,
        requirements: [
          {
            id: SetupRequirementId.github_token,
            status: SetupRequirementStatus.missing,
            severity: SetupSeverity.required,
            actions: [{ kind: SetupActionKind.open_settings, target: 'github' }],
          },
        ],
      },
    })

    const { result } = renderHook(() => useSetupStatus())

    expect(result.current.isComplete).toBe(false)
    expect(result.current.unmet).toHaveLength(1)
    expect(result.current.unmet[0]?.copy.title).toContain('GitHub')
    expect(result.current.unmet[0]?.action).toEqual({
      kind: 'settings',
      section: 'github',
      label: 'Connect GitHub',
    })
  })

  it('keeps satisfied requirements in `items` but out of `unmet`', () => {
    mockSession({
      user: null,
      expiresAt: null,
      providers: [],
      setup: {
        complete: true,
        requirements: [
          {
            id: SetupRequirementId.github_token,
            status: SetupRequirementStatus.satisfied,
            severity: SetupSeverity.required,
          },
        ],
      },
    })

    const { result } = renderHook(() => useSetupStatus())

    expect(result.current.items).toHaveLength(1)
    expect(result.current.unmet).toHaveLength(0)
  })

  it('gives admins the org-wide route when the backend offers it', () => {
    mockIsAdmin(true)
    mockSession({
      user: null,
      expiresAt: null,
      providers: [],
      setup: {
        complete: false,
        requirements: [
          {
            id: SetupRequirementId.github_token,
            status: SetupRequirementStatus.missing,
            severity: SetupSeverity.required,
            actions: [
              { kind: SetupActionKind.admin_route, target: '/admin/integrations/github' },
              { kind: SetupActionKind.open_settings, target: 'github' },
            ],
          },
        ],
      },
    })

    const { result } = renderHook(() => useSetupStatus())

    expect(result.current.unmet[0]?.action?.kind).toBe('route')
  })
})
