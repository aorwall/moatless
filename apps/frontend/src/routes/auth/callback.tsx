import { createFileRoute, useNavigate } from '@tanstack/react-router'
import { useEffect } from 'react'

export const Route = createFileRoute('/auth/callback')({
  validateSearch: (search): { success?: string } => ({
    success: (search as { success?: string }).success,
  }),
  component: AuthCallback,
})

function AuthCallback() {
  const navigate = useNavigate()
  const { success } = Route.useSearch()

  useEffect(() => {
    if (success === 'true') {
      // OAuth succeeded, redirect immediately
      navigate({ to: '/' })
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
