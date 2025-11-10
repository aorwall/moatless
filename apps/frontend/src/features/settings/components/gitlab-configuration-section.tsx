import { ExternalLink, Loader2, LogOut, Plug } from 'lucide-react'
import { useState } from 'react'
import { toast } from 'sonner'
import { useAuth } from '@/features/auth/hooks/use-auth'
import {
  useDisconnectGitLab,
  useGitLabConfig,
  useSaveGitLabPAT,
} from '@/features/settings/hooks/use-gitlab-connection'
import {
  ConfigurationGroup,
  ConfigurationItem,
  ConfigurationSectionWrapper,
} from '@/lib/components/configuration'
import { Alert, AlertDescription, AlertTitle } from '@/lib/components/ui/alert'
import { Button } from '@/lib/components/ui/button'
import { Input } from '@/lib/components/ui/input'
import { getErrorMessage } from '@/lib/utils/error'

export function GitLabConfigurationSection() {
  const { user } = useAuth()
  const { data: gitlabConfig, isLoading: configLoading, error: gitlabError } = useGitLabConfig()
  const disconnectGitLab = useDisconnectGitLab()
  const savePAT = useSaveGitLabPAT()
  const [pat, setPat] = useState('')

  // Handle disconnect button click
  const handleDisconnect = () => {
    disconnectGitLab.mutate(undefined, {
      onSuccess: () => {
        toast.success('Disconnected from GitLab')
      },
      onError: (error) => {
        toast.error('Failed to disconnect', {
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

  if (gitlabError) {
    return (
      <Alert variant="destructive">
        <AlertTitle>Error</AlertTitle>
        <AlertDescription>Failed to load GitLab settings. Please try again later.</AlertDescription>
      </Alert>
    )
  }

  // If user is authenticated via GitLab OAuth, show read-only info
  if (user?.provider === 'gitlab') {
    return (
      <ConfigurationSectionWrapper>
        <ConfigurationGroup title="GitLab Configuration" showSeparator={false}>
          <Alert>
            <AlertTitle>Authenticated via GitLab OAuth</AlertTitle>
            <AlertDescription>
              <div className="space-y-2">
                <p className="text-sm">
                  You are authenticated with GitLab. Your credentials are automatically used for git
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

  // For non-GitLab authenticated users, show service connection UI
  const isConnected = gitlabConfig?.connected ?? false

  return (
    <ConfigurationSectionWrapper>
      <ConfigurationGroup title="GitLab Configuration" showSeparator={false}>
        <p className="text-sm text-muted-foreground mb-4">
          Connect your GitLab account to access private repositories and push changes
        </p>

        {/* Connection Status */}
        {isConnected && (
          <Alert className="mb-4">
            <Plug className="h-4 w-4" />
            <AlertTitle>Connected to GitLab</AlertTitle>
            <AlertDescription className="space-y-2">
              <p className="text-sm">Your GitLab account is connected via Personal Access Token.</p>
              {gitlabConfig?.gitlabUsername && (
                <p className="text-sm">
                  <strong>Username:</strong> {gitlabConfig.gitlabUsername}
                </p>
              )}
              {gitlabConfig?.connectedAt && (
                <p className="text-sm text-muted-foreground">
                  Connected on {new Date(gitlabConfig.connectedAt).toLocaleDateString()}
                </p>
              )}
            </AlertDescription>
          </Alert>
        )}

        {/* PAT Input (when not connected) */}
        {!isConnected && (
          <div className="space-y-4">
            <ConfigurationItem
              label="Personal Access Token"
              description="Use a GitLab PAT for authentication"
            >
              <div className="flex gap-2">
                <Input
                  type="password"
                  placeholder="Enter your GitLab personal access token"
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
                      href="https://gitlab.com/-/user_settings/personal_access_tokens"
                      target="_blank"
                      rel="noopener noreferrer"
                      className="text-blue-600 hover:underline inline-flex items-center gap-1"
                    >
                      GitLab Settings → Access Tokens
                      <ExternalLink className="h-3 w-3" />
                    </a>
                  </li>
                  <li>Click "Add new token"</li>
                  <li>Give your token a descriptive name</li>
                  <li>
                    Select the following scopes: <strong>read_api</strong>,{' '}
                    <strong>read_repository</strong>, <strong>write_repository</strong>
                  </li>
                  <li>Click "Create personal access token" and copy it</li>
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
              disabled={disconnectGitLab.isPending}
              variant="outline"
              size="sm"
            >
              {disconnectGitLab.isPending ? (
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
