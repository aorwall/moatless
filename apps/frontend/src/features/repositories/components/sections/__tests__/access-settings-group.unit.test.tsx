import userEvent from '@testing-library/user-event'
import type { ComponentProps } from 'react'
import { describe, expect, it, vi } from 'vitest'
import { render, screen } from '@/test-utils/render'
import { AccessSettingsGroup } from '../access-settings-group'

const teams = [{ id: 'team-1', name: 'Platform', slug: 'platform' }]
const users = [{ id: 'user-1', login: 'alice', name: 'Alice Example' }]

function renderGroup(overrides: Partial<ComponentProps<typeof AccessSettingsGroup>> = {}) {
  const props: ComponentProps<typeof AccessSettingsGroup> = {
    scope: 'user',
    teamId: 'team-1',
    ownerUserId: 'user-1',
    teams,
    users,
    teamsLoading: false,
    accessStatus: 'ok',
    accessCheckedAt: '2026-07-24T10:00:00Z',
    accessError: null,
    isDirty: false,
    isVerifying: false,
    onScopeChange: vi.fn(),
    onTeamChange: vi.fn(),
    onVerify: vi.fn(),
    ...overrides,
  }
  render(<AccessSettingsGroup {...props} />)
  return props
}

describe('AccessSettingsGroup', () => {
  it('shows the immutable owner and current verification state', () => {
    renderGroup()

    expect(screen.getByLabelText('Repository owner')).toHaveValue('Alice Example (alice)')
    expect(screen.getByText('Verified')).toBeInTheDocument()
    expect(screen.getByText(/Last checked:/)).toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'Verify access' })).toBeEnabled()
  })

  it('falls back to the owner UUID when the user is unavailable', () => {
    renderGroup({ users: [] })

    expect(screen.getByLabelText('Repository owner')).toHaveValue('user-1')
  })

  it('requires a team for team scope', () => {
    renderGroup({ scope: 'team', teamId: null })

    expect(screen.getByText('Select a team to save.')).toBeInTheDocument()
  })

  it('disables verification while persisted fields are dirty', () => {
    renderGroup({ isDirty: true })

    expect(screen.getByRole('button', { name: 'Verify access' })).toBeDisabled()
    expect(screen.getByText('Save or discard changes before verifying access.')).toBeInTheDocument()
  })

  it('shows provider failures and invokes manual verification', async () => {
    const user = userEvent.setup()
    const onVerify = vi.fn()
    renderGroup({
      accessStatus: 'forbidden',
      accessError: 'No access to repository',
      onVerify,
    })

    expect(screen.getByText('No access')).toBeInTheDocument()
    expect(screen.getByText('No access to repository')).toBeInTheDocument()
    await user.click(screen.getByRole('button', { name: 'Verify access' }))
    expect(onVerify).toHaveBeenCalledOnce()
  })
})
