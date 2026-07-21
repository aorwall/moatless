import type { LinkProps } from '@tanstack/react-router'

/**
 * User-facing copy for a setup requirement.
 *
 * The backend sends ids and machine-readable action targets only, so every
 * string a user reads is owned here. `adminAction.to` is a *typed* route —
 * never the backend's `target` string, which would defeat TanStack Router's
 * link typing and let the server dictate frontend URLs.
 */
export interface SetupRequirementCopy {
  title: string
  description: string
  /** For non-admins: opens the settings dialog at this section id. */
  userAction?: { label: string; settingsSection: string }
  /** For admins: an org-wide fix that resolves the requirement for everyone. */
  adminAction?: { label: string; to: NonNullable<LinkProps['to']> }
  learnMore?: { label: string; href: string }
}

export const SETUP_REQUIREMENT_COPY = {
  github_token: {
    title: 'GitHub access is not set up',
    description:
      'Moatless needs a GitHub token to read repositories and start tasks. Without one, repositories will not load.',
    userAction: { label: 'Connect GitHub', settingsSection: 'github' },
    adminAction: { label: 'Configure for everyone', to: '/admin/integrations/github' },
    learnMore: {
      label: 'Create a token',
      href: 'https://github.com/settings/tokens/new',
    },
  },
} as const satisfies Record<string, SetupRequirementCopy>

export type KnownSetupRequirementId = keyof typeof SETUP_REQUIREMENT_COPY

/**
 * Unknown ids get generic copy with no CTA rather than being dropped: silently
 * hiding an unrecognised *required* item would leave the app reporting
 * "setup incomplete" with nothing on screen to explain it.
 */
export function getSetupRequirementCopy(id: string): SetupRequirementCopy {
  return (
    SETUP_REQUIREMENT_COPY[id as KnownSetupRequirementId] ?? {
      title: 'A setup step is incomplete',
      description: `This deployment reports an unfinished setup step ("${id}") that this version of the app does not recognise. Update the app or contact an administrator.`,
    }
  )
}
