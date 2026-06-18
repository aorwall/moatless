import type { ReactNode } from 'react'

import { queryClient } from '@/lib/api/query-client'
import { useServerStatus } from '@/lib/api/server-status'
import { ServerDown } from './server-down'

/**
 * Root-level gate that renders the full-page `ServerDown` screen whenever the
 * backend is unreachable, instead of the app (or the login page). "Try again"
 * re-runs all queries; a successful response clears the flag via the
 * `QueryCache` `onSuccess` handler and the app renders again.
 */
export function ServerDownGate({ children }: { children: ReactNode }) {
  const { isUnavailable } = useServerStatus()

  if (isUnavailable) {
    return <ServerDown onRetry={() => queryClient.refetchQueries()} />
  }

  return <>{children}</>
}
