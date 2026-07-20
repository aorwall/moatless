import type { AuthModeResponse } from '@moatless/api-client'
import { unwrapResponse } from '@moatless/api-client/custom-instance'
import { authModeHandler } from '@moatless/api-client/sandbox/auth/auth'
import { queryOptions, useQuery } from '@tanstack/react-query'

import { queryKeys } from '@/lib/api/query-keys'

/**
 * Colocated `queryKey` + `queryFn` for `GET /api/v1/auth/mode`.
 *
 * A deployment's auth mode is fixed for the life of the process, so this never
 * goes stale. Deliberately NOT tagged `serverHealthProbe` — that meta flag is
 * reserved for `/auth/me`, the single request that drives `ServerDownGate`.
 */
export function authModeQueryOptions() {
  return queryOptions({
    queryKey: queryKeys.auth.mode(),
    queryFn: () => unwrapResponse<AuthModeResponse>(authModeHandler()),
    staleTime: Number.POSITIVE_INFINITY,
    retry: false,
  })
}

/** How this deployment authenticates users. */
export function useAuthMode() {
  const { data, isLoading, error } = useQuery(authModeQueryOptions())

  return {
    mode: data?.mode ?? null,
    provider: data?.provider ?? null,
    loginUrl: data?.loginUrl ?? null,
    isPasswordMode: data?.mode === 'password',
    isLoading,
    error,
  }
}
