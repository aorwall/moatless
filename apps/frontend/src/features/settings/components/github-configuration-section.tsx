import type { GitHubConfigResponse } from '@moatless/api-client/sandbox/model'
import { useQueryClient } from '@tanstack/react-query'
import { ExternalLink, Loader2, LogOut, Plug } from 'lucide-react'
import { useEffect, useRef, useState } from 'react'
import { toast } from 'sonner'
import { useAuth } from '@/features/auth/hooks/use-auth'
import { GitHubPatForm } from '@/features/settings/components/github-pat-form'
import { GitHubPatHelp } from '@/features/settings/components/github-pat-help'
import {
  useDisconnectGitHub,
  useGetGitHubOAuthUrl,
  useGitHubConfig,
  useRemoveGitHubPATOverride,
  useSaveGitHubPAT,
} from '@/features/settings/hooks/use-github-connection'
import { queryKeys } from '@/lib/api/query-keys'
import {
  ConfigurationGroup,
  ConfigurationItem,
  ConfigurationSectionWrapper,
} from '@/lib/components/configuration'
import { Alert, AlertDescription, AlertTitle } from '@/lib/components/ui/alert'
import { Button } from '@/lib/components/ui/button'
import { getErrorMessage } from '@/lib/utils/error'

/**
 * PAT override section for GitHub-OAuth users.
 * Allows saving/updating/removing a PAT that takes precedence over the OAuth token.
 */
function GitHubPATOverrideSection({
  githubConfig,
}: {
  githubConfig: GitHubConfigResponse | undefined
}) {
  const savePAT = useSaveGitHubPAT()
  const removePATOverride = useRemoveGitHubPATOverride()

  const handleSavePAT = async (token: string) => {
    const result = await savePAT.mutateAsync(
      { token },
      {
        onError: (error) => {
          toast.error('Failed to save Personal Access Token', {
            description: getErrorMessage(error),
          })
        },
      },
    )
    toast.success('Personal Access Token saved successfully')
    return result
  }

  const handleRemovePATOverride = () => {
    removePATOverride.mutate(undefined, {
      onSuccess: () => {
        toast.success('PAT override removed, reverted to OAuth token')
      },
      onError: (error) => {
        toast.error('Failed to remove PAT override', {
          description: getErrorMessage(error),
        })
      },
    })
  }

  const hasOverride = githubConfig?.hasPatOverride ?? false

  return (
    <ConfigurationItem
      label="Personal Access Token Override"
      description="Used only for repositories persisted with User scope. A PAT overrides your OAuth token; Team and Global repositories use their bound GitHub App installation."
    >
      <div className="space-y-3">
        {hasOverride && (
          <Alert>
            <Plug className="h-4 w-4" />
            <AlertTitle>PAT Override Active</AlertTitle>
            <AlertDescription>
              A Personal Access Token is being used instead of your OAuth token for git operations.
              {githubConfig?.login && (
                <>
                  {' '}
                  <strong>Username:</strong> {githubConfig.login}
                </>
              )}
            </AlertDescription>
          </Alert>
        )}
        <GitHubPatForm
          hasToken={hasOverride}
          onSave={handleSavePAT}
          isSaving={savePAT.isPending}
          onRemove={handleRemovePATOverride}
          isRemoving={removePATOverride.isPending}
          removeLabel="Remove PAT Override"
        />
      </div>
    </ConfigurationItem>
  )
}

export function GitHubConfigurationSection() {
  const { user } = useAuth()
  const { data: githubConfig, isLoading: configLoading, error: githubError } = useGitHubConfig()
  const disconnectGitHub = useDisconnectGitHub()
  const savePAT = useSaveGitHubPAT()
  const [isConnecting, setIsConnecting] = useState(false)
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

    const handleFocus = () => {
      queryClient
        .invalidateQueries({ queryKey: queryKeys.settings.github() })
        .then(() => {
          const latestConfig = queryClient.getQueryData<GitHubConfigResponse>(
            queryKeys.settings.github(),
          )
          if (latestConfig?.connected) {
            setIsConnecting(false)
            popupRef.current = null
            toast.success('Successfully connected to GitHub!')
          }
        })
        .catch(() => {
          // The query exposes its own error state; focus detection is best-effort.
        })
    }

    window.addEventListener('focus', handleFocus)

    return () => {
      window.removeEventListener('focus', handleFocus)
    }
  }, [isConnecting, queryClient])

  // Poll to detect when popup window is closed
  useEffect(() => {
    if (!(isConnecting && popupRef.current)) {
      return
    }

    const checkPopupClosed = setInterval(() => {
      if (popupRef.current?.closed) {
        clearInterval(checkPopupClosed)

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

  const handleConnect = async () => {
    setIsConnecting(true)
    try {
      const result = await fetchOAuthUrl()
      if (result.data?.url) {
        const width = 600
        const height = 700
        const left = window.screen.width / 2 - width / 2
        const top = window.screen.height / 2 - height / 2

        popupRef.current = window.open(
          result.data.url,
          'github-oauth',
          `width=${width},height=${height},left=${left},top=${top}`,
        )

        toast.info('Continue in the new window to connect your GitHub account')
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

  const handleDisconnect = () => {
    disconnectGitHub.mutate(undefined, {
      onSuccess: () => {
        toast.success('GitHub account disconnected')
      },
      onError: (error) => {
        toast.error('Failed to disconnect GitHub', {
          description: getErrorMessage(error),
        })
      },
    })
  }

  const handleSavePAT = async (token: string) => {
    const result = await savePAT.mutateAsync(
      { token },
      {
        onError: (error) => {
          toast.error('Failed to save Personal Access Token', {
            description: getErrorMessage(error),
          })
        },
      },
    )
    toast.success('Personal Access Token saved successfully')
    return result
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

  // If user is authenticated via GitHub OAuth, show auth info + PAT override option
  if (user?.provider === 'github') {
    return (
      <ConfigurationSectionWrapper>
        <ConfigurationGroup title="GitHub Configuration" showSeparator={false}>
          <Alert>
            <AlertTitle>Authenticated via GitHub OAuth</AlertTitle>
            <AlertDescription>
              <div className="space-y-2">
                <p className="text-sm">
                  These credentials apply only to repositories with User scope that you own.
                  {githubConfig?.hasPatOverride
                    ? ' Your Personal Access Token overrides OAuth.'
                    : ' Your OAuth token is used because no PAT override is configured.'}
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

          <GitHubPATOverrideSection githubConfig={githubConfig} />
        </ConfigurationGroup>
      </ConfigurationSectionWrapper>
    )
  }

  // For non-GitHub authenticated users, show service connection UI
  const isConnected = githubConfig?.connected ?? false
  const hasOAuthEnabled = githubConfig?.tokenType === 'service-oauth'

  return (
    <ConfigurationSectionWrapper>
      <ConfigurationGroup title="GitHub Configuration" showSeparator={false}>
        <p className="text-sm text-muted-foreground mb-4">
          Connect credentials for repositories with User scope that you own. A Personal Access Token
          overrides OAuth; Team and Global repositories use GitHub App installations.
        </p>

        {/* Connection Status */}
        {isConnected && (
          <Alert className="mb-4">
            <Plug className="h-4 w-4" />
            <AlertTitle>Connected to GitHub</AlertTitle>
            <AlertDescription className="space-y-2">
              <p className="text-sm">
                Your GitHub account is connected via{' '}
                {githubConfig?.tokenType === 'service-oauth' ? 'OAuth' : 'Personal Access Token'}.
              </p>
              {githubConfig?.login && (
                <p className="text-sm">
                  <strong>Username:</strong> {githubConfig.login}
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
              <GitHubPatForm hasToken={false} onSave={handleSavePAT} isSaving={savePAT.isPending} />
            </ConfigurationItem>

            <GitHubPatHelp />
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
