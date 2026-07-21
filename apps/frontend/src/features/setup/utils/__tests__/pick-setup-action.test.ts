import {
  SetupActionKind,
  type SetupRequirement,
  SetupRequirementId,
  SetupRequirementStatus,
  SetupSeverity,
} from '@moatless/api-client'
import { describe, expect, it } from 'vitest'

import type { SetupRequirementCopy } from '../../copy/setup-requirement-copy'
import { pickSetupAction } from '../pick-setup-action'

const COPY: SetupRequirementCopy = {
  title: 'GitHub access is not set up',
  description: '…',
  userAction: { label: 'Connect GitHub', settingsSection: 'github' },
  adminAction: { label: 'Configure for everyone', to: '/admin/integrations/github' },
}

function requirement(kinds: SetupActionKind[]): SetupRequirement {
  return {
    id: SetupRequirementId.github_token,
    status: SetupRequirementStatus.missing,
    severity: SetupSeverity.required,
    actions: kinds.map((kind) => ({ kind, target: 'ignored-by-the-frontend' })),
  }
}

describe('pickSetupAction', () => {
  it('prefers the org-wide route for admins, since it fixes it for everyone', () => {
    const action = pickSetupAction(
      requirement([SetupActionKind.admin_route, SetupActionKind.open_settings]),
      COPY,
      true,
    )
    expect(action).toEqual({
      kind: 'route',
      to: '/admin/integrations/github',
      label: 'Configure for everyone',
    })
  })

  it('sends non-admins to their own settings', () => {
    const action = pickSetupAction(requirement([SetupActionKind.open_settings]), COPY, false)
    expect(action).toEqual({ kind: 'settings', section: 'github', label: 'Connect GitHub' })
  })

  it('falls back to settings for an admin when no admin route is offered', () => {
    const action = pickSetupAction(requirement([SetupActionKind.open_settings]), COPY, true)
    expect(action?.kind).toBe('settings')
  })

  it('returns null when the backend offers nothing actionable', () => {
    expect(pickSetupAction(requirement([]), COPY, true)).toBeNull()
  })

  it('returns null when the frontend has no copy for the offered action', () => {
    const bareCopy: SetupRequirementCopy = { title: 't', description: 'd' }
    expect(
      pickSetupAction(requirement([SetupActionKind.open_settings]), bareCopy, false),
    ).toBeNull()
  })

  it("never uses the backend's target string as the destination", () => {
    // The route comes from typed frontend copy; a server-supplied string would
    // defeat TanStack Router's link typing.
    const action = pickSetupAction(requirement([SetupActionKind.admin_route]), COPY, true)
    expect(action).toEqual({
      kind: 'route',
      to: '/admin/integrations/github',
      label: 'Configure for everyone',
    })
  })
})
