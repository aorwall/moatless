import { createFileRoute } from '@tanstack/react-router'
import { Loader2 } from 'lucide-react'
import { useEffect } from 'react'
import { useExchangeGitHubToken } from '@/features/settings'
import { Alert, AlertDescription, AlertTitle } from '@/lib/components/ui/alert'
import { Button } from '@/lib/components/ui/button'
import { getErrorMessage } from '@/lib/utils/error'
import { oauthCallbackSearchSchema } from '@/lib/utils/oauth-callback-search-schema'

export const Route = createFileRoute('/oauth/github/callback')({
  validateSearch: oauthCallbackSearchSchema,
  component: GitHubOAuthCallback,
})

function closeWindow() {
  window.close()
}

function GitHubOAuthCallback() {
  const { code, state } = Route.useSearch()
  const exchangeToken = useExchangeGitHubToken()

  useEffect(() => {
    if (!(code && state)) {
      return
    }

    // Exchange code for token
    exchangeToken.mutate(
      { code, state },
      {
        onSuccess: () => {
          // Close the popup window after a short delay
          setTimeout(() => {
            window.close()
          }, 2000)
        },
      },
    )
  }, [code, state, exchangeToken.mutate])

  if (code && state && !exchangeToken.isSuccess && !exchangeToken.isError) {
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

  if (exchangeToken.isSuccess) {
    return (
      <div className="flex items-center justify-center min-h-screen">
        <div className="text-center max-w-md mx-auto p-6">
          <div className="text-6xl mb-4">✅</div>
          <h1 className="text-2xl font-semibold mb-2">Successfully Connected!</h1>
          <p className="text-muted-foreground mb-6">
            Your GitHub account has been connected. This window will close automatically.
          </p>
          <Button onClick={closeWindow}>Close Window</Button>
        </div>
      </div>
    )
  }

  const errorMessage =
    code && state && exchangeToken.error
      ? getErrorMessage(exchangeToken.error)
      : 'Missing code or state parameter from OAuth callback'

  return (
    <div className="flex items-center justify-center min-h-screen">
      <div className="max-w-md mx-auto p-6">
        <Alert variant="destructive">
          <AlertTitle>Connection Failed</AlertTitle>
          <AlertDescription>
            <p className="mb-4">{errorMessage}</p>
            <Button onClick={closeWindow} variant="outline">
              Close Window
            </Button>
          </AlertDescription>
        </Alert>
      </div>
    </div>
  )
}
