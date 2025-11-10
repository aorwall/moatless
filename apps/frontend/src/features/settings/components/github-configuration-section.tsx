import { useQueryClient } from '@tanstack/react-query'
import { ExternalLink, Loader2, LogOut, Plug } from 'lucide-react'
import { useEffect, useRef, useState } from 'react'
import { toast } from 'sonner'
import { useAuth } from '@/features/auth/hooks/use-auth'
import {
  useDisconnectGitHub,
  useGetGitHubOAuthUrl,
  useGitHubConfig,
  useSaveGitHubPAT,
} from '@/features/settings'
import { queryKeys } from '@/lib/api/query-keys'
import {
  ConfigurationGroup,
  ConfigurationItem,
  ConfigurationSectionWrapper,
} from '@/lib/components/configuration'
import { Alert, AlertDescription, AlertTitle } from '@/lib/components/ui/alert'
import { Button } from '@/lib/components/ui/button'
import { Input } from '@/lib/components/ui/input'
import { getErrorMessage } from '@/lib/utils/error'

export function GitHubConfigurationSection() {
  const { user } = useAuth()
  const { data: githubConfig, isLoading: configLoading, error: githubError } = useGitHubConfig()
  const disconnectGitHub = useDisconnectGitHub()
  const savePAT = useSaveGitHubPAT()
  const [isConnecting, setIsConnecting] = useState(false)
  const [pat, setPat] = useState('')
  const queryClient = useQueryClient()
  const popupRef = useRef<Window | null>(null)

  // Fetch OAuth URL only when user clicks connect button
  const { refetch: fetchOAuthUrl } = useGetGitHubOAuthUrl({
    enabled: false,
  })

  // Listen for window focus to detect when OAuth popup closes
  useEffect(() => {
    if (!isConnecting) {
      return
    }

    // When OAuth popup is opened, listen for window focus to detect when user returns
    const handleFocus = () => {
      // Invalidate GitHub config to refetch connection status
      queryClient.invalidateQueries({ queryKey: queryKeys.settings.github() })
    }

    window.addEventListener('focus', handleFocus)

    return () => {
      window.removeEventListener('focus', handleFocus)
    }
  }, [isConnecting, queryClient])

  // Reset connecting state and show success when connection is detected
  useEffect(() => {
    // When config updates and shows connected, reset connecting state
    if (isConnecting && githubConfig?.connected) {
      setIsConnecting(false)
      popupRef.current = null
      toast.success('Successfully connected to GitHub!')
    }
  }, [isConnecting, githubConfig?.connected])

  // Poll to detect when popup window is closed
  useEffect(() => {
    if (!(isConnecting && popupRef.current)) {
      return
    }

    const checkPopupClosed = setInterval(() => {
      if (popupRef.current?.closed) {
        clearInterval(checkPopupClosed)

        // Check if connection succeeded
        if (!githubConfig?.connected) {
          setIsConnecting(false)
          toast.info('Authorization cancelled', {
            description: 'You can try again or use a Personal Access Token instead',
          })
        }

        popupRef.current = null
      }
    }, 500)

    return () => {
      clearInterval(checkPopupClosed)
    }
  }, [isConnecting, githubConfig?.connected])

  // Handle connect button click
  const handleConnect = async () => {
    setIsConnecting(true)
    try {
      const result = await fetchOAuthUrl()
      if (result.data?.authUrl) {
        // Open OAuth URL in new window
        const width = 600
        const height = 700
        const left = window.screen.width / 2 - width / 2
        const top = window.screen.height / 2 - height / 2

        popupRef.current = window.open(
          result.data.authUrl,
          'github-oauth',
          `width=${width},height=${height},left=${left},top=${top}`,
        )

        toast.info('Continue in the new window to connect your GitHub account')
        // Don't set isConnecting=false yet - keep it true while OAuth is in progress
        // The focus event will trigger refetch, and we'll set it to false when connection succeeds
      } else {
        setIsConnecting(false)
      }
    } catch (error) {
      toast.error('Failed to initiate GitHub OAuth', {
        description: getErrorMessage(error),
      })
      setIsConnecting(false)
    }
  }

  // Handle disconnect button click
  const handleDisconnect = () => {
    disconnectGitHub.mutate(undefined, {
      onSuccess: () => {
        toast.success('GitHub account disconnected')
        setPat('') // Clear PAT input
      },
      onError: (error) => {
        toast.error('Failed to disconnect GitHub', {
          description: getErrorMessage(error),
        })
      },
    })
  }

  // Handle save PAT button click
  const handleSavePAT = () => {
    if (!pat.trim()) {
      toast.error('Please enter a Personal Access Token')
      return
    }

    savePAT.mutate(
      { accessToken: pat },
      {
        onSuccess: () => {
          toast.success('Personal Access Token saved successfully')
          setPat('') // Clear input after successful save
        },
        onError: (error) => {
          toast.error('Failed to save Personal Access Token', {
            description: getErrorMessage(error),
          })
        },
      },
    )
  }

  if (configLoading) {
    return (
      <div className="flex items-center justify-center h-32">
        <Loader2 className="h-8 w-8 animate-spin" data-testid="loading-spinner" />
      </div>
    )
  }

  if (githubError) {
    return (
      <Alert variant="destructive">
        <AlertTitle>Error</AlertTitle>
        <AlertDescription>Failed to load GitHub settings. Please try again later.</AlertDescription>
      </Alert>
    )
  }

  // If user is authenticated via GitHub OAuth, show read-only info
  if (user?.provider === 'github') {
    return (
      <ConfigurationSectionWrapper>
        <ConfigurationGroup title="GitHub Configuration" showSeparator={false}>
          <Alert>
            <AlertTitle>Authenticated via GitHub OAuth</AlertTitle>
            <AlertDescription>
              <div className="space-y-2">
                <p className="text-sm">
                  You are authenticated with GitHub. Your credentials are automatically used for git
                  operations.
                </p>
                {user.name && (
                  <p className="text-sm">
                    <strong>Name:</strong> {user.name}
                  </p>
                )}
                <p className="text-sm">
                  <strong>Username:</strong> {user.login}
                </p>
                {user.email && (
                  <p className="text-sm">
                    <strong>Email:</strong> {user.email}
                  </p>
                )}
              </div>
            </AlertDescription>
          </Alert>
        </ConfigurationGroup>
      </ConfigurationSectionWrapper>
    )
  }

  // For non-GitHub authenticated users, show service connection UI
  const isConnected = githubConfig?.connected ?? false
  const hasOAuthEnabled = githubConfig?.oauthEnabled ?? false

  return (
    <ConfigurationSectionWrapper>
      <ConfigurationGroup title="GitHub Configuration" showSeparator={false}>
        <p className="text-sm text-muted-foreground mb-4">
          Connect your GitHub account to access private repositories and push changes
        </p>

        {/* Connection Status */}
        {isConnected && (
          <Alert className="mb-4">
            <Plug className="h-4 w-4" />
            <AlertTitle>Connected to GitHub</AlertTitle>
            <AlertDescription className="space-y-2">
              <p className="text-sm">
                Your GitHub account is connected via{' '}
                {githubConfig?.method === 'service-oauth' ? 'OAuth' : 'Personal Access Token'}.
              </p>
              {githubConfig?.githubLogin && (
                <p className="text-sm">
                  <strong>Username:</strong> {githubConfig.githubLogin}
                </p>
              )}
              {githubConfig?.connectedAt && (
                <p className="text-sm text-muted-foreground">
                  Connected on {new Date(githubConfig.connectedAt).toLocaleDateString()}
                </p>
              )}
            </AlertDescription>
          </Alert>
        )}

        {/* OAuth Connection (if enabled and not connected) */}
        {!isConnected && hasOAuthEnabled && (
          <div className="space-y-4">
            <ConfigurationItem
              label="OAuth Connection"
              description="Connect using GitHub OAuth for seamless authentication"
            >
              <Button
                onClick={handleConnect}
                disabled={isConnecting}
                variant="outline"
                className="w-full sm:w-auto"
              >
                {isConnecting ? (
                  <>
                    <Loader2 className="mr-2 h-4 w-4 animate-spin" />
                    Connecting...
                  </>
                ) : (
                  <>
                    <ExternalLink className="mr-2 h-4 w-4" />
                    Connect with GitHub
                  </>
                )}
              </Button>
            </ConfigurationItem>

            <div className="relative">
              <div className="absolute inset-0 flex items-center">
                <span className="w-full border-t" />
              </div>
              <div className="relative flex justify-center text-xs uppercase">
                <span className="bg-background px-2 text-muted-foreground">Or</span>
              </div>
            </div>
          </div>
        )}

        {/* PAT Input (always available when not connected) */}
        {!isConnected && (
          <div className="space-y-4">
            <ConfigurationItem
              label="Personal Access Token"
              description="Alternatively, use a GitHub PAT for authentication"
            >
              <div className="flex gap-2">
                <Input
                  type="password"
                  placeholder="Enter your GitHub personal access token"
                  value={pat}
                  onChange={(e) => setPat(e.target.value)}
                  className="flex-1 font-mono"
                />
                <Button
                  onClick={handleSavePAT}
                  disabled={savePAT.isPending || !pat.trim()}
                  variant="outline"
                >
                  {savePAT.isPending ? <Loader2 className="h-4 w-4 animate-spin" /> : 'Save'}
                </Button>
              </div>
            </ConfigurationItem>

            <Alert>
              <AlertTitle>How to create a Personal Access Token</AlertTitle>
              <AlertDescription className="space-y-2">
                <ol className="list-decimal list-inside space-y-1 text-sm">
                  <li>
                    Go to{' '}
                    <a
                      href="https://github.com/settings/tokens/new"
                      target="_blank"
                      rel="noopener noreferrer"
                      className="text-blue-600 hover:underline inline-flex items-center gap-1"
                    >
                      GitHub Settings → Personal access tokens
                      <ExternalLink className="h-3 w-3" />
                    </a>
                  </li>
                  <li>Click "Generate new token (classic)"</li>
                  <li>Give your token a descriptive name</li>
                  <li>Select the "repo" scope for repository access</li>
                  <li>Click "Generate token" and copy it</li>
                </ol>
              </AlertDescription>
            </Alert>
          </div>
        )}

        {/* Disconnect Button (when connected) */}
        {isConnected && (
          <div className="flex justify-end pt-4">
            <Button
              onClick={handleDisconnect}
              disabled={disconnectGitHub.isPending}
              variant="outline"
              size="sm"
            >
              {disconnectGitHub.isPending ? (
                <Loader2 className="mr-2 h-4 w-4 animate-spin" />
              ) : (
                <LogOut className="mr-2 h-4 w-4" />
              )}
              Disconnect
            </Button>
          </div>
        )}
      </ConfigurationGroup>
    </ConfigurationSectionWrapper>
  )
}
