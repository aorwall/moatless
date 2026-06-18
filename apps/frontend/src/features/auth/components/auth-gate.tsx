import { Navigate } from '@tanstack/react-router'
import type { ReactNode } from 'react'

import { useAuth } from '@/features/auth/hooks/use-auth'
import { useServerStatus } from '@/lib/api/server-status'

/**
 * Gates the authenticated app behind a valid session. Redirects to `/login`
 * on 401 — but NOT while the backend is unreachable (503 / network failure).
 *
 * During downtime the session check (`/me`) fails with a non-auth error, so
 * without this guard the gate would bounce the user to a login page that also
 * can't reach the backend. Instead we let the root-level `ServerDownGate`
 * surface the "Server unavailable" screen. We check both the session error
 * (`isServerUnavailable` from `useAuth`) and the global store
 * (`useServerStatus`) so a 503 surfaced by any other request is honored too.
 */
export function AuthGate({ children }: { children: ReactNode }) {
  const { isAuthenticated, isLoading, isServerUnavailable } = useAuth()
  const { isUnavailable } = useServerStatus()

  if (isLoading) {
    return (
      <div className="flex min-h-screen items-center justify-center">
        <div className="text-muted-foreground">Loading...</div>
      </div>
    )
  }

  if (!(isAuthenticated || isServerUnavailable || isUnavailable)) {
    return <Navigate to="/login" />
  }

  return children
}
