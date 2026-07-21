import { SetupRequirementStatus, SetupSeverity } from '@moatless/api-client'
import { fireEvent, render, screen } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'

import type { SetupItem } from '../../hooks/use-setup-status'
import { useSetupStatus } from '../../hooks/use-setup-status'
import { SetupRequirementsBanner } from '../setup-requirements-banner'

const { mockOpenSettings } = vi.hoisted(() => ({ mockOpenSettings: vi.fn() }))

vi.mock('../../hooks/use-setup-status', () => ({ useSetupStatus: vi.fn() }))
vi.mock('@/features/settings/contexts/settings-dialog-provider', () => ({
  useSettingsDialog: () => ({ openSettings: mockOpenSettings, closeSettings: vi.fn() }),
}))
vi.mock('@tanstack/react-router', () => ({
  Link: ({ to, children }: { to: string; children: React.ReactNode }) => (
    <a href={to} data-testid="route-link">
      {children}
    </a>
  ),
}))

function item(overrides: Partial<SetupItem> = {}): SetupItem {
  return {
    id: 'github_token',
    status: SetupRequirementStatus.missing,
    severity: SetupSeverity.required,
    copy: {
      title: 'GitHub access is not set up',
      description: 'Moatless needs a GitHub token.',
      userAction: { label: 'Connect GitHub', settingsSection: 'github' },
    },
    action: { kind: 'settings', section: 'github', label: 'Connect GitHub' },
    ...overrides,
  }
}

function mockUnmet(unmet: SetupItem[]) {
  vi.mocked(useSetupStatus).mockReturnValue({
    isComplete: unmet.length === 0,
    isLoading: false,
    items: unmet,
    unmet,
  })
}

describe('SetupRequirementsBanner', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    sessionStorage.clear()
  })

  it('renders nothing when setup is complete', () => {
    mockUnmet([])
    const { container } = render(<SetupRequirementsBanner />)
    expect(container).toBeEmptyDOMElement()
  })

  it('renders the requirement copy when something is missing', () => {
    mockUnmet([item()])
    render(<SetupRequirementsBanner />)
    expect(screen.getByText('GitHub access is not set up')).toBeInTheDocument()
  })

  it('opens the settings dialog at the right section for a non-admin CTA', () => {
    mockUnmet([item()])
    render(<SetupRequirementsBanner />)
    fireEvent.click(screen.getByRole('button', { name: 'Connect GitHub' }))
    expect(mockOpenSettings).toHaveBeenCalledWith('github')
  })

  it('renders the admin CTA as a link to the admin route', () => {
    mockUnmet([
      item({
        action: {
          kind: 'route',
          to: '/admin/integrations/github',
          label: 'Configure for everyone',
        },
      }),
    ])
    render(<SetupRequirementsBanner />)
    expect(screen.getByTestId('route-link')).toHaveAttribute('href', '/admin/integrations/github')
  })

  it('renders copy without a button when there is no actionable fix', () => {
    mockUnmet([item({ action: null })])
    render(<SetupRequirementsBanner />)
    expect(screen.queryByRole('button')).not.toBeInTheDocument()
  })

  describe('dismissal', () => {
    it('does not offer to dismiss a required item', () => {
      mockUnmet([item()])
      render(<SetupRequirementsBanner />)
      expect(screen.queryByRole('button', { name: /dismiss/i })).not.toBeInTheDocument()
    })

    it('hides a recommended item once dismissed', () => {
      mockUnmet([item({ severity: SetupSeverity.recommended })])
      const { rerender } = render(<SetupRequirementsBanner />)

      fireEvent.click(screen.getByRole('button', { name: /dismiss/i }))
      rerender(<SetupRequirementsBanner />)

      expect(screen.queryByText('GitHub access is not set up')).not.toBeInTheDocument()
    })
  })
})
