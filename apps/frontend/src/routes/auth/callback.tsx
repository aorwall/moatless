import { createFileRoute, useNavigate } from '@tanstack/react-router'
import { useEffect } from 'react'
import { z } from 'zod'
import { consumeReturnTo } from '@/features/auth/utils/return-to'

const callbackSearchSchema = z.object({
  success: z.string().optional().catch(undefined),
})

export const Route = createFileRoute('/auth/callback')({
  validateSearch: callbackSearchSchema,
  component: AuthCallback,
})

function AuthCallback() {
  const navigate = useNavigate()
  const { success } = Route.useSearch()

  useEffect(() => {
    if (success === 'true') {
      // OAuth succeeded — return to the page the user was heading for when
      // AuthGate bounced them (validated same-origin path), or the dashboard.
      // A hard navigation is fine here: it picks up the fresh session cookie.
      const returnTo = consumeReturnTo()
      if (returnTo) {
        window.location.replace(returnTo)
      } else {
        navigate({ to: '/' }).catch((error: unknown) => {
          // Navigation failure after OAuth — not much we can do; log for debugging
          if (import.meta.env.DEV) {
            // biome-ignore lint/suspicious/noConsole: dev-only diagnostic
            console.error('Navigation failed after OAuth callback', error)
          }
        })
      }
    }
  }, [success, navigate])

  const API_BASE_URL = import.meta.env.PROD ? '' : (import.meta.env.VITE_API_URL ?? '')

  const handleRetry = () => {
    window.location.href = `${API_BASE_URL}/api/v1/auth/login`
  }

  if (success === 'true') {
    return (
      <div className="flex items-center justify-center min-h-screen">
        <div className="text-center">
          <div className="text-muted-foreground">Redirecting...</div>
        </div>
      </div>
    )
  }

  return (
    <div className="flex items-center justify-center min-h-screen">
      <div className="text-center max-w-md mx-auto p-6">
        <div className="text-6xl mb-4">⚠️</div>
        <h1 className="text-2xl font-semibold mb-2">Authentication Failed</h1>
        <p className="text-muted-foreground mb-6">
          Unable to complete GitHub authentication. Please try again.
        </p>
        <button
          type="button"
          onClick={handleRetry}
          className="px-4 py-2 bg-primary text-primary-foreground rounded-md hover:bg-primary/90"
        >
          Retry Login
        </button>
      </div>
    </div>
  )
}
