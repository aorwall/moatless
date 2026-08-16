import type { GitHubProviderTokenStatusResponse } from '@moatless/api-client/sandbox/model'
import { Loader2, LogOut, Plug } from 'lucide-react'
import { toast } from 'sonner'
import { useAuth } from '@/features/auth/hooks/use-auth'
import { GitHubPatForm } from '@/features/settings/components/github-pat-form'
import { GitHubPatHelp } from '@/features/settings/components/github-pat-help'
import {
  useDisconnectGitHub,
  useGitHubConfig,
  useRemoveGitHubPATOverride,
  useSaveGitHubPAT,
} from '@/features/settings/hooks/use-github-connection'
import {
  ConfigurationGroup,
  ConfigurationItem,
  ConfigurationSectionWrapper,
} from '@/lib/components/configuration'
import { Alert, AlertDescription, AlertTitle } from '@/lib/components/ui/alert'
import { Button } from '@/lib/components/ui/button'
import { getErrorMessage } from '@/lib/utils/error'

/**
 * PAT override section for GitHub App users.
 * Allows saving/updating/removing a PAT that takes precedence over the App user token.
 */
function GitHubPATOverrideSection({
  githubConfig,
}: {
  githubConfig: GitHubProviderTokenStatusResponse | undefined
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
        toast.success('PAT override removed, reverted to GitHub App token')
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
      description="Used only for repositories persisted with User scope. A PAT overrides your GitHub App user token; Global repositories use their bound GitHub App installation."
    >
      <div className="space-y-3">
        {hasOverride && (
          <Alert>
            <Plug className="h-4 w-4" />
            <AlertTitle>PAT Override Active</AlertTitle>
            <AlertDescription>
              A Personal Access Token is being used instead of your GitHub App user token for git
              operations.
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

  // GitHub App users can override their user access token with a PAT.
  if (user?.provider === 'github_app') {
    return (
      <ConfigurationSectionWrapper>
        <ConfigurationGroup title="GitHub Configuration" showSeparator={false}>
          <Alert>
            <AlertTitle>Authenticated via GitHub App</AlertTitle>
            <AlertDescription>
              <div className="space-y-2">
                <p className="text-sm">
                  These credentials apply only to repositories with User scope that you own.
                  {githubConfig?.hasPatOverride
                    ? ' Your Personal Access Token overrides the GitHub App user token.'
                    : ' Your GitHub App user token is used because no PAT override is configured.'}
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

  // Other auth providers can configure a PAT for user-scoped repositories.
  const isConnected = githubConfig?.connected ?? false

  return (
    <ConfigurationSectionWrapper>
      <ConfigurationGroup title="GitHub Configuration" showSeparator={false}>
        <p className="text-sm text-muted-foreground mb-4">
          Connect credentials for repositories with User scope that you own. A Personal Access Token
          is used for these repositories; Global repositories use GitHub App installations.
        </p>

        {/* Connection Status */}
        {isConnected && (
          <Alert className="mb-4">
            <Plug className="h-4 w-4" />
            <AlertTitle>Connected to GitHub</AlertTitle>
            <AlertDescription className="space-y-2">
              <p className="text-sm">A Personal Access Token is configured.</p>
              {githubConfig?.login && (
                <p className="text-sm">
                  <strong>Username:</strong> {githubConfig.login}
                </p>
              )}
            </AlertDescription>
          </Alert>
        )}

        {/* PAT input */}
        {!isConnected && (
          <div className="space-y-4">
            <ConfigurationItem
              label="Personal Access Token"
              description="Use a GitHub PAT for user-scoped repository access"
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
