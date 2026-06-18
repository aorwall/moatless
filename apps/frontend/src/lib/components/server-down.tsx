import { RefreshCw, ServerOff } from 'lucide-react'

import { Button } from './ui/button'

interface ServerDownProps {
  onRetry: () => void
}

/**
 * Full-page screen shown when the backend is unreachable (HTTP 503 or a
 * network-unreachable failure). Mirrors the layout of `full-page-error.tsx`
 * and `lib/pages/404.tsx`. No "Dashboard" link — navigation is pointless while
 * the server is down; "Try again" re-runs the failed requests.
 */
export function ServerDown({ onRetry }: ServerDownProps) {
  return (
    <div className="flex min-h-screen items-center justify-center p-8">
      <div className="flex max-w-md flex-col items-center space-y-4 text-center">
        <div className="rounded-full bg-muted p-4">
          <ServerOff className="h-8 w-8 text-muted-foreground" />
        </div>

        <div className="space-y-1">
          <h2 className="text-xl font-semibold">Server unavailable</h2>
          <p className="text-sm text-muted-foreground">
            We can't reach the Moatless server right now — it may be restarting or temporarily down.
          </p>
        </div>

        <Button variant="outline" size="sm" onClick={onRetry} className="mt-2">
          <RefreshCw className="mr-2 h-4 w-4" />
          Try again
        </Button>
      </div>
    </div>
  )
}
