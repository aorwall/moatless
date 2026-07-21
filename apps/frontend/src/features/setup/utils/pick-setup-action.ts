import { SetupActionKind, type SetupRequirement } from '@moatless/api-client'
import type { LinkProps } from '@tanstack/react-router'

import type { SetupRequirementCopy } from '../copy/setup-requirement-copy'

export type ResolvedSetupAction =
  | { kind: 'settings'; section: string; label: string }
  | { kind: 'route'; to: NonNullable<LinkProps['to']>; label: string }

/**
 * Choose the single CTA to show for a requirement.
 *
 * The backend's `actions` array is a *capability hint* — it says which routes
 * are meaningful for this caller (admin-only actions are already filtered out
 * server-side). The label and destination come from the frontend's own copy
 * entry, so `action.target` is only ever matched against, never navigated to.
 *
 * Returns `null` when neither side offers a usable action; the banner then
 * renders copy without a button.
 */
export function pickSetupAction(
  requirement: SetupRequirement,
  copy: SetupRequirementCopy,
  isAdmin: boolean,
): ResolvedSetupAction | null {
  const actions = requirement.actions ?? []
  const offersAdminRoute = actions.some((a) => a.kind === SetupActionKind.admin_route)
  const offersSettings = actions.some((a) => a.kind === SetupActionKind.open_settings)

  // An org-wide fix resolves the requirement for every user, so it wins.
  if (isAdmin && offersAdminRoute && copy.adminAction) {
    return { kind: 'route', to: copy.adminAction.to, label: copy.adminAction.label }
  }

  if (offersSettings && copy.userAction) {
    return {
      kind: 'settings',
      section: copy.userAction.settingsSection,
      label: copy.userAction.label,
    }
  }

  return null
}
