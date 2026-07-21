import {
  type SetupRequirement,
  SetupRequirementStatus,
  type SetupSeverity,
} from '@moatless/api-client'
import { useQuery } from '@tanstack/react-query'
import { useMemo } from 'react'

import { authSessionQueryOptions, useAuth } from '@/features/auth'
import { getSetupRequirementCopy, type SetupRequirementCopy } from '../copy/setup-requirement-copy'
import { pickSetupAction, type ResolvedSetupAction } from '../utils/pick-setup-action'

export interface SetupItem {
  id: string
  status: SetupRequirementStatus
  severity: SetupSeverity
  copy: SetupRequirementCopy
  action: ResolvedSetupAction | null
}

/**
 * Projects `/auth/me`'s `setup` block into renderable items.
 *
 * Reads the same query as `useAuth` (deduped, one cache entry) rather than
 * extending it — `useAuth` is mocked wholesale in several tests, and every
 * field added there churns those mocks.
 */
export function useSetupStatus() {
  const { data: session, isLoading } = useQuery(authSessionQueryOptions())
  const { isAdmin } = useAuth()

  // An absent block means an older backend, a still-loading session, or an
  // unauthenticated caller — none of which should flash a banner. Same
  // reasoning as `githubConnected ?? true` in use-auth.ts.
  const isComplete = session?.setup?.complete ?? true
  const requirements = useMemo(
    () => session?.setup?.requirements ?? [],
    [session?.setup?.requirements],
  )

  // Cheap enough to derive during render — there are only ever a handful.
  const items = requirements.map((requirement) => toItem(requirement, isAdmin))
  const unmet = items.filter((item) => item.status !== SetupRequirementStatus.satisfied)

  return { isComplete, isLoading, items, unmet }
}

function toItem(requirement: SetupRequirement, isAdmin: boolean): SetupItem {
  const copy = getSetupRequirementCopy(requirement.id)
  return {
    id: requirement.id,
    status: requirement.status,
    severity: requirement.severity,
    copy,
    action: pickSetupAction(requirement, copy, isAdmin),
  }
}
