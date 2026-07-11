import { createFileRoute } from '@tanstack/react-router'
import { Loader2 } from 'lucide-react'
import { useEffect, useState } from 'react'
import { useExchangeGitHubToken } from '@/features/settings'
import { Alert, AlertDescription, AlertTitle } from '@/lib/components/ui/alert'
import { Button } from '@/lib/components/ui/button'
import { getErrorMessage } from '@/lib/utils/error'
import { oauthCallbackSearchSchema } from '@/lib/utils/oauth-callback-search-schema'

export const Route = createFileRoute('/oauth/github/callback')({
  validateSearch: oauthCallbackSearchSchema,
  component: GitHubOAuthCallback,
})

function GitHubOAuthCallback() {
  const { code, state } = Route.useSearch()
  const exchangeToken = useExchangeGitHubToken()
  const [status, setStatus] = useState<'loading' | 'success' | 'error'>('loading')
  const [errorMessage, setErrorMessage] = useState<string | null>(null)

  // biome-ignore lint/correctness/useExhaustiveDependencies: exchangeToken.mutate is stable from react-query
  useEffect(() => {
    // Validate we have code and state
    if (!(code && state)) {
      setStatus('error')
      setErrorMessage('Missing code or state parameter from OAuth callback')
      return
    }

    // Exchange code for token
    exchangeToken.mutate(
      { code, state },
      {
        onSuccess: () => {
          setStatus('success')
          // Close the popup window after a short delay
          setTimeout(() => {
            window.close()
          }, 2000)
        },
        onError: (error) => {
          setStatus('error')
          setErrorMessage(getErrorMessage(error))
        },
      },
    )
  }, [code, state])

  const handleClose = () => {
    window.close()
  }

  if (status === 'loading') {
    return (
      <div className="flex items-center justify-center min-h-screen">
        <div className="text-center">
          <Loader2 className="h-12 w-12 animate-spin mx-auto mb-4" />
          <div className="text-lg font-medium">Connecting to GitHub...</div>
          <div className="text-sm text-muted-foreground mt-2">
            Please wait while we complete the authentication
          </div>
        </div>
      </div>
    )
  }

  if (status === 'success') {
    return (
      <div className="flex items-center justify-center min-h-screen">
        <div className="text-center max-w-md mx-auto p-6">
          <div className="text-6xl mb-4">✅</div>
          <h1 className="text-2xl font-semibold mb-2">Successfully Connected!</h1>
          <p className="text-muted-foreground mb-6">
            Your GitHub account has been connected. This window will close automatically.
          </p>
          <Button onClick={handleClose}>Close Window</Button>
        </div>
      </div>
    )
  }

  return (
    <div className="flex items-center justify-center min-h-screen">
      <div className="max-w-md mx-auto p-6">
        <Alert variant="destructive">
          <AlertTitle>Connection Failed</AlertTitle>
          <AlertDescription>
            <p className="mb-4">
              {errorMessage || 'Unable to connect to GitHub. Please try again.'}
            </p>
            <Button onClick={handleClose} variant="outline">
              Close Window
            </Button>
          </AlertDescription>
        </Alert>
      </div>
    </div>
  )
}
