import type { ProviderConnection, SessionResponse } from '@moatless/api-client'
import { unwrapResponse } from '@moatless/api-client/custom-instance'
import { meHandler } from '@moatless/api-client/sandbox/auth/auth'
import { queryOptions, useQuery } from '@tanstack/react-query'

import { queryKeys } from '@/lib/api/query-keys'
import { isServerUnavailable } from '@/lib/api/server-status'

const API_BASE_URL = import.meta.env.PROD ? '' : (import.meta.env.VITE_API_URL ?? '')

/**
 * Colocated `queryKey` + `queryFn` (+ defaults) for the current auth session.
 * Shared between the `useAuth` hook and route loaders (e.g. the admin guard) so
 * cache access stays typed and the key can never drift between call sites.
 */
export function authSessionQueryOptions() {
  return queryOptions({
    queryKey: queryKeys.auth.session(),
    queryFn: () => unwrapResponse<SessionResponse>(meHandler()),
    retry: false,
    staleTime: 5 * 60 * 1000,
    // This session probe (`GET /api/v1/auth/me`) is the single source of truth
    // for backend reachability: only a 503/network failure here flips the
    // global `ServerDownGate`. A 503 from any other endpoint fails only its own
    // part of the UI. See `query-client.ts` `QueryCache` callbacks.
    meta: { serverHealthProbe: true },
  })
}

/**
 * Hook to get the current authenticated user.
 * The root-level AuthGate handles redirecting to /login on 401.
 */
export function useAuth() {
  const { data: session, isLoading, error, refetch } = useQuery(authSessionQueryOptions())

  /**
   * Logout by navigating to the logout endpoint
   * Uses GET request via window.location to allow proper redirect handling
   * For Auth0 users, backend will redirect to Auth0 logout, then back to app
   * For GitHub users, backend will return JSON and redirect to home
   */
  const logout = () => {
    window.location.href = `${API_BASE_URL}/api/v1/auth/logout`
  }

  const providers: ProviderConnection[] = session?.providers ?? []
  const githubProvider = providers.find((p) => p.provider === 'github')
  // GitHub only appears in `providers` for users who connected it, so the
  // `?? true` default keeps the banner hidden for everyone else — and avoids
  // flashing it before the session loads.
  const githubConnected = githubProvider?.connected ?? true
  // Codex only appears in `providers` for users who connected it, so the
  // `?? true` default keeps the banner hidden for everyone else.
  const codexProvider = providers.find((p) => p.provider === 'codex')
  const codexConnected = codexProvider?.connected ?? true

  return {
    user: session?.user ?? null,
    role: session?.user?.role ?? null,
    isAdmin: session?.user?.role === 'admin',
    expiresAt: session?.expiresAt ?? null,
    isAuthenticated: !!session?.user,
    isLoading,
    isServerUnavailable: isServerUnavailable(error),
    refetch,
    providers,
    githubConnected,
    codexConnected,
    logout,
  }
}
