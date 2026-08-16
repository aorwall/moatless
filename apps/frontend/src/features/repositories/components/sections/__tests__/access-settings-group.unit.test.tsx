import type { ComponentProps } from 'react'
import { describe, expect, it, vi } from 'vitest'
import { render, screen } from '@/test-utils/render'
import { AccessSettingsGroup } from '../access-settings-group'

const users = [{ id: 'user-1', login: 'alice', name: 'Alice Example' }]

function renderGroup(overrides: Partial<ComponentProps<typeof AccessSettingsGroup>> = {}) {
  const props: ComponentProps<typeof AccessSettingsGroup> = {
    scope: 'user',
    ownerUserId: 'user-1',
    users,
    onScopeChange: vi.fn(),
    ...overrides,
  }
  render(<AccessSettingsGroup {...props} />)
  return props
}

describe('AccessSettingsGroup', () => {
  it('shows the immutable owner', () => {
    renderGroup()

    expect(screen.getByLabelText('Repository owner')).toHaveValue('Alice Example (alice)')
  })

  it('falls back to the owner UUID when the user is unavailable', () => {
    renderGroup({ users: [] })

    expect(screen.getByLabelText('Repository owner')).toHaveValue('user-1')
  })

  it('names the viewer as the credential, not the owner', () => {
    // A shared repository does not lend its owner's access to whoever opens it.
    renderGroup({ scope: 'global', ownerUserId: null })

    expect(screen.getByText('Your own GitHub credential')).toBeInTheDocument()
  })
})
