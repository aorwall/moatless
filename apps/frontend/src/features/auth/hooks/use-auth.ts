import type { SessionResponse } from '@moatless/api-client'
import { unwrapResponse } from '@moatless/api-client/custom-instance'
import { meHandler } from '@moatless/api-client/sandbox/auth/auth'
import { useQuery } from '@tanstack/react-query'

const API_BASE_URL = import.meta.env.PROD ? '' : (import.meta.env.VITE_API_URL ?? '')

/**
 * Hook to get the current authenticated user.
 * The root-level AuthGate handles redirecting to /login on 401.
 */
export function useAuth() {
  const { data: session, isLoading } = useQuery({
    queryKey: ['auth', 'session'],
    queryFn: () => unwrapResponse<SessionResponse>(meHandler()),
    retry: false,
    staleTime: 5 * 60 * 1000,
  })

  /**
   * Logout by navigating to the logout endpoint
   * Uses GET request via window.location to allow proper redirect handling
   * For Auth0 users, backend will redirect to Auth0 logout, then back to app
   * For GitHub users, backend will return JSON and redirect to home
   */
  const logout = () => {
    window.location.href = `${API_BASE_URL}/api/v1/auth/logout`
  }

  return {
    user: session?.user ?? null,
    role: session?.user?.role ?? null,
    isAdmin: session?.user?.role === 'admin',
    expiresAt: session?.expiresAt ?? null,
    isAuthenticated: !!session,
    isLoading,
    logout,
  }
}
